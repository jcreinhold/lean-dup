use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use serde::Serialize;

use crate::EvalSuite;
use crate::eval::labels::{GoldLabels, load_builtin};
use crate::eval::scorer_ablations::{self, ScorerAblationVariantReport};
use crate::eval::scoring::{
    CountMetric, EvaluationMetrics, GoldPair, ObservedPair, ObservedRun, RecallAtK, TimingMetrics, score_run,
};
use crate::eval::search_dataset;
use crate::eval::stage_metrics::SemanticVerificationStageMetrics;
use crate::eval::vector_search::{self, VectorSearchRequest};
use lean_dup_diagnostics::perf;
use lean_dup_diagnostics::progress::Reporter;
use lean_dup_index::{
    DeclarationHandle, HydratedDeclaration, IndexBuildKind, IndexBuildRequest, IndexReference, IndexStore, OpenedIndex,
};
use lean_dup_project::{WorkspaceRequest, resolve, resolve_project_mathlib};
use lean_dup_search::{
    SearchObservation, SearchObservationRequest, SearchScoringVariant, SearchTrackedPair, SearchVectorCandidateStatus,
    SearchVectorCandidateSummary, SearchVectorEligibilitySummary, VECTOR_CANDIDATE_TOP_K, observe_search,
    observe_search_with_progress, rescore_observation,
};
use lean_dup_worker::{Fingerprints, WorkerClient};

use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct EvalRequest {
    pub suite: EvalSuite,
    pub workspace: Option<PathBuf>,
    pub mathlib_workspace: Option<PathBuf>,
    pub manual_module: Option<String>,
    pub k_values: Vec<usize>,
    pub write_search_dataset: bool,
    pub write_scorer_ablations: bool,
    pub vector_search: Option<VectorSearchRequest>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalOutput {
    pub status: String,
    pub suite: String,
    pub scorer_version: String,
    pub metrics: EvaluationMetrics,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_dataset_artifact: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scorer_ablation_artifact: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_search_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_search_artifact: Option<PathBuf>,
    #[serde(skip)]
    pub scorer_ablations: Vec<ScorerAblationVariantReport>,
    #[serde(skip)]
    pub vector_search_metrics: Option<EvaluationMetrics>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub runs: Vec<EvaluationRunReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvaluationRunReport {
    pub suite: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scorer_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<EvaluationMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_search_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_search_artifact: Option<PathBuf>,
    pub manual: bool,
    #[serde(skip)]
    pub scorer_ablations: Vec<ScorerAblationVariantReport>,
    #[serde(skip)]
    pub vector_search_metrics: Option<EvaluationMetrics>,
}

struct SuiteDefinition {
    suite: EvalSuite,
    workspace: PathBuf,
    module_root: String,
    origin: String,
    external: Option<ExternalSuiteIndex>,
    mathlib_source_override: Option<PathBuf>,
    build_before_index: bool,
    require_oleans: bool,
}

struct ExternalSuiteIndex {
    workspace: PathBuf,
    module_root: String,
    label: String,
    origin: String,
    require_oleans: bool,
}

struct SuiteIndexRequest<'a> {
    workspace_root: &'a Path,
    module_root: &'a str,
    label: &'a str,
    origin: &'a str,
    require_oleans: bool,
    build_before_index: bool,
    kind: IndexBuildKind,
}

pub fn run(request: EvalRequest, reporter: &mut Reporter) -> Result<EvalOutput> {
    if request.suite == EvalSuite::ProductionGate {
        return run_production_gate(request, reporter);
    }
    run_single(request, reporter)
}

fn run_single(request: EvalRequest, reporter: &mut Reporter) -> Result<EvalOutput> {
    if request.suite == EvalSuite::VectorFixture {
        return run_vector_fixture(request, reporter);
    }

    let total_started = Instant::now();
    let write_search_dataset = request.write_search_dataset;
    let write_scorer_ablations = request.write_scorer_ablations;
    let labels = load_builtin(request.suite)?;
    let definition = suite_definition(&request);
    let k_values = suite_k_values(request.suite, &request.k_values);
    let cache_root = cache_root_for(&definition, reporter)?;

    let index_started = Instant::now();
    let local_label = format!("eval-{}-workspace", definition.suite.as_str());
    let local = build_or_load_index(
        SuiteIndexRequest {
            workspace_root: &definition.workspace,
            module_root: &definition.module_root,
            label: &local_label,
            origin: &definition.origin,
            require_oleans: definition.require_oleans,
            build_before_index: definition.build_before_index,
            kind: IndexBuildKind::Local,
        },
        &cache_root,
        reporter,
    )?;
    let handles = local.all_handles()?;
    let workspace_rows = local.hydrate(&handles)?;
    let external =
        match &definition.external {
            Some(external) if definition.suite == EvalSuite::ManualMathlib => Some(
                build_or_load_project_mathlib_index(&definition, external, &cache_root, reporter)?,
            ),
            Some(external) => Some(build_or_load_index(
                SuiteIndexRequest {
                    workspace_root: &external.workspace,
                    module_root: &external.module_root,
                    label: &external.label,
                    origin: &external.origin,
                    require_oleans: external.require_oleans,
                    build_before_index: definition.build_before_index && !external.require_oleans,
                    kind: IndexBuildKind::External,
                },
                &cache_root,
                reporter,
            )?),
            None => None,
        };
    let index_load_ms = index_started.elapsed().as_millis();

    let tracked_pairs = tracked_pairs(&labels);
    let retrieval_started = Instant::now();
    let base_output = match &external {
        Some(external) => observe_search(SearchObservationRequest {
            workspace: &workspace_rows,
            comparison_indexes: std::slice::from_ref(external),
            tracked_pairs: &tracked_pairs,
            scoring_variant: SearchScoringVariant::SymbolicOnly,
            vector_candidates: None,
        })?,
        None => observe_search(SearchObservationRequest {
            workspace: &workspace_rows,
            comparison_indexes: &[],
            tracked_pairs: &tracked_pairs,
            scoring_variant: SearchScoringVariant::SymbolicOnly,
            vector_candidates: None,
        })?,
    };
    let base_retrieval_ms = retrieval_started.elapsed().as_millis();

    let vector_query_count = workspace_rows.len();
    let vector_corpus_count = match &external {
        Some(external) => external.all_handles()?.len(),
        None => workspace_rows.len(),
    };
    let vector_search_request = request
        .vector_search
        .as_ref()
        .map(|request| request.to_search_request(&labels.suite));
    let mut vector_partial = None;
    let (output, retrieval_ms) = if let Some(vector_request) = vector_search_request.as_ref() {
        reporter.event(
            "eval.vector.bounds",
            None,
            None,
            format!(
                "checking vector validation bounds for {} queries and {} corpus declarations",
                vector_query_count, vector_corpus_count
            ),
        );
        if let Some(reason) = vector_budget_violation(
            request.vector_search.as_ref().expect("vector request is present"),
            vector_query_count,
            vector_corpus_count,
            total_started,
        ) {
            let root = repo_root();
            let summary = budget_vector_summary(
                request.vector_search.as_ref().expect("vector request is present"),
                vector_query_count,
                vector_corpus_count,
                reason.clone(),
            );
            reporter.event(
                "eval.vector.artifact",
                None,
                None,
                "writing partial vector validation artifact",
            );
            vector_partial = Some(vector_search::partial_report(
                vector_search::VectorSearchPartialReportRun {
                    repo_root: &root,
                    suite: &labels.suite,
                    status: "budget-exceeded",
                    reason,
                    request: request.vector_search.as_ref(),
                    symbolic_baseline: None,
                    vector_candidates: summary,
                    started: total_started,
                },
            )?);
            (base_output.clone(), base_retrieval_ms)
        } else {
            reporter.event(
                "eval.vector.search",
                None,
                None,
                "running hidden vector candidate generation",
            );
            let started = Instant::now();
            let output = match &external {
                Some(external) => observe_search_with_progress(
                    SearchObservationRequest {
                        workspace: &workspace_rows,
                        comparison_indexes: std::slice::from_ref(external),
                        tracked_pairs: &tracked_pairs,
                        scoring_variant: SearchScoringVariant::SymbolicOnly,
                        vector_candidates: Some(vector_request),
                    },
                    reporter,
                )?,
                None => observe_search_with_progress(
                    SearchObservationRequest {
                        workspace: &workspace_rows,
                        comparison_indexes: &[],
                        tracked_pairs: &tracked_pairs,
                        scoring_variant: SearchScoringVariant::SymbolicOnly,
                        vector_candidates: Some(vector_request),
                    },
                    reporter,
                )?,
            };
            (output, started.elapsed().as_millis())
        }
    } else {
        (base_output.clone(), base_retrieval_ms)
    };
    let scorer_version = output.scoring.version.to_owned();

    let base_observed = ObservedRun {
        suite: labels.suite.clone(),
        pairs: observed_pairs(&base_output),
        visible_groups: CountMetric {
            found: base_output.visible_groups_found,
            total: base_output.visible_groups_total,
        },
        probe_unavailable: CountMetric { found: 0, total: 0 },
        semantic_verification: SemanticVerificationStageMetrics {
            semantic_reranking: base_output.semantic_reranking.clone(),
            obligation_yield: base_output.semantic_obligation_yield.clone(),
            ..SemanticVerificationStageMetrics::default()
        },
        timings: TimingMetrics {
            index_load_ms,
            retrieval_ms: base_retrieval_ms,
            probe_ms: 0,
            total_ms: total_started.elapsed().as_millis(),
        },
        peak_memory_bytes: perf::peak_rss_bytes(),
    };
    let baseline_metrics = score_run(&labels, &base_observed, &k_values);

    let observed = ObservedRun {
        suite: labels.suite.clone(),
        pairs: observed_pairs(&output),
        visible_groups: CountMetric {
            found: output.visible_groups_found,
            total: output.visible_groups_total,
        },
        probe_unavailable: CountMetric { found: 0, total: 0 },
        semantic_verification: SemanticVerificationStageMetrics {
            semantic_reranking: output.semantic_reranking.clone(),
            obligation_yield: output.semantic_obligation_yield.clone(),
            ..SemanticVerificationStageMetrics::default()
        },
        timings: TimingMetrics {
            index_load_ms,
            retrieval_ms,
            probe_ms: 0,
            total_ms: total_started.elapsed().as_millis(),
        },
        peak_memory_bytes: perf::peak_rss_bytes(),
    };
    let metrics = score_run(&labels, &observed, &k_values);
    enforce_suite_gates(&definition, &labels, &baseline_metrics)?;
    let search_dataset_artifact = if write_search_dataset {
        let dataset = search_dataset::build(&labels.suite, &labels, &output);
        Some(search_dataset::write_default_artifact(&repo_root(), &dataset)?)
    } else {
        None
    };
    let scorer_ablations = if write_scorer_ablations {
        scorer_ablation_variants(&labels, &output, &k_values, index_load_ms, reporter)
    } else {
        Vec::new()
    };
    let scorer_ablation_artifact = if write_scorer_ablations {
        let report = scorer_ablations::report(
            &labels.suite,
            &scorer_version,
            output.semantic_reranking.clone(),
            output.semantic_obligation_yield.clone(),
            scorer_ablations.clone(),
            Vec::new(),
        );
        Some(scorer_ablations::write_default_artifact(&repo_root(), &report)?)
    } else {
        None
    };
    let vector_search = if let Some(outcome) = vector_partial {
        Some(outcome)
    } else if let Some(vector_request) = request.vector_search.as_ref() {
        let root = repo_root();
        if let Some(reason) =
            vector_budget_violation(vector_request, vector_query_count, vector_corpus_count, total_started)
        {
            reporter.event(
                "eval.vector.artifact",
                None,
                None,
                "writing budget-exceeded vector artifact",
            );
            Some(vector_search::partial_report(
                vector_search::VectorSearchPartialReportRun {
                    repo_root: &root,
                    suite: &labels.suite,
                    status: "budget-exceeded",
                    reason,
                    request: Some(vector_request),
                    symbolic_baseline: None,
                    vector_candidates: output.retrieval.vector_candidates.clone(),
                    started: total_started,
                },
            )?)
        } else {
            reporter.event("eval.vector.artifact", None, None, "writing vector validation artifact");
            Some(vector_search::report(vector_search::VectorSearchReportRun {
                repo_root: &root,
                suite: &labels.suite,
                labels: &labels,
                observation: &output,
                symbolic_baseline: &baseline_metrics,
                vector_metrics: &metrics,
                scorer_version: &scorer_version,
                k_values: &k_values,
                request: Some(vector_request),
                started: total_started,
            })?)
        }
    } else {
        None
    };

    Ok(EvalOutput {
        status: "ok".to_owned(),
        suite: labels.suite,
        scorer_version,
        metrics,
        search_dataset_artifact,
        scorer_ablation_artifact,
        vector_search_status: vector_search.as_ref().map(|outcome| outcome.status.clone()),
        vector_search_artifact: vector_search.as_ref().map(|outcome| outcome.artifact.clone()),
        scorer_ablations,
        vector_search_metrics: vector_search.and_then(|outcome| outcome.metrics),
        runs: Vec::new(),
    })
}

fn run_vector_fixture(request: EvalRequest, reporter: &mut Reporter) -> Result<EvalOutput> {
    let total_started = Instant::now();
    let write_search_dataset = request.write_search_dataset;
    let write_scorer_ablations = request.write_scorer_ablations;
    let labels = load_builtin(EvalSuite::VectorFixture)?;
    let k_values = suite_k_values(EvalSuite::VectorFixture, &request.k_values);
    let workspace_rows = vector_fixture_declarations();
    let tracked_pairs = tracked_pairs(&labels);

    let retrieval_started = Instant::now();
    let base_output = observe_search(SearchObservationRequest {
        workspace: &workspace_rows,
        comparison_indexes: &[],
        tracked_pairs: &tracked_pairs,
        scoring_variant: SearchScoringVariant::SymbolicOnly,
        vector_candidates: None,
    })?;
    let base_retrieval_ms = retrieval_started.elapsed().as_millis();

    let vector_search_request = request
        .vector_search
        .as_ref()
        .map(|request| request.to_search_request(&labels.suite));
    let mut vector_partial = None;
    let (output, retrieval_ms) = if let Some(vector_request) = vector_search_request.as_ref() {
        reporter.event(
            "eval.vector.bounds",
            None,
            None,
            format!(
                "checking vector validation bounds for {} queries and {} corpus declarations",
                workspace_rows.len(),
                workspace_rows.len()
            ),
        );
        if let Some(reason) = vector_budget_violation(
            request.vector_search.as_ref().expect("vector request is present"),
            workspace_rows.len(),
            workspace_rows.len(),
            total_started,
        ) {
            let root = repo_root();
            let summary = budget_vector_summary(
                request.vector_search.as_ref().expect("vector request is present"),
                workspace_rows.len(),
                workspace_rows.len(),
                reason.clone(),
            );
            reporter.event(
                "eval.vector.artifact",
                None,
                None,
                "writing partial vector validation artifact",
            );
            vector_partial = Some(vector_search::partial_report(
                vector_search::VectorSearchPartialReportRun {
                    repo_root: &root,
                    suite: &labels.suite,
                    status: "budget-exceeded",
                    reason,
                    request: request.vector_search.as_ref(),
                    symbolic_baseline: None,
                    vector_candidates: summary,
                    started: total_started,
                },
            )?);
            (base_output.clone(), base_retrieval_ms)
        } else {
            reporter.event(
                "eval.vector.search",
                None,
                None,
                "running hidden vector candidate generation",
            );
            let started = Instant::now();
            let output = observe_search_with_progress(
                SearchObservationRequest {
                    workspace: &workspace_rows,
                    comparison_indexes: &[],
                    tracked_pairs: &tracked_pairs,
                    scoring_variant: SearchScoringVariant::SymbolicOnly,
                    vector_candidates: Some(vector_request),
                },
                reporter,
            )?;
            (output, started.elapsed().as_millis())
        }
    } else {
        (base_output.clone(), base_retrieval_ms)
    };
    let scorer_version = output.scoring.version.to_owned();

    let base_observed = ObservedRun {
        suite: labels.suite.clone(),
        pairs: observed_pairs(&base_output),
        visible_groups: CountMetric {
            found: base_output.visible_groups_found,
            total: base_output.visible_groups_total,
        },
        probe_unavailable: CountMetric { found: 0, total: 0 },
        semantic_verification: SemanticVerificationStageMetrics {
            semantic_reranking: base_output.semantic_reranking.clone(),
            obligation_yield: base_output.semantic_obligation_yield.clone(),
            ..SemanticVerificationStageMetrics::default()
        },
        timings: TimingMetrics {
            index_load_ms: 0,
            retrieval_ms: base_retrieval_ms,
            probe_ms: 0,
            total_ms: total_started.elapsed().as_millis(),
        },
        peak_memory_bytes: perf::peak_rss_bytes(),
    };
    let baseline_metrics = score_run(&labels, &base_observed, &k_values);

    let observed = ObservedRun {
        suite: labels.suite.clone(),
        pairs: observed_pairs(&output),
        visible_groups: CountMetric {
            found: output.visible_groups_found,
            total: output.visible_groups_total,
        },
        probe_unavailable: CountMetric { found: 0, total: 0 },
        semantic_verification: SemanticVerificationStageMetrics {
            semantic_reranking: output.semantic_reranking.clone(),
            obligation_yield: output.semantic_obligation_yield.clone(),
            ..SemanticVerificationStageMetrics::default()
        },
        timings: TimingMetrics {
            index_load_ms: 0,
            retrieval_ms,
            probe_ms: 0,
            total_ms: total_started.elapsed().as_millis(),
        },
        peak_memory_bytes: perf::peak_rss_bytes(),
    };
    let metrics = score_run(&labels, &observed, &k_values);

    let search_dataset_artifact = if write_search_dataset {
        let dataset = search_dataset::build(&labels.suite, &labels, &output);
        Some(search_dataset::write_default_artifact(&repo_root(), &dataset)?)
    } else {
        None
    };
    let scorer_ablations = if write_scorer_ablations {
        scorer_ablation_variants(&labels, &output, &k_values, 0, reporter)
    } else {
        Vec::new()
    };
    let scorer_ablation_artifact = if write_scorer_ablations {
        let report = scorer_ablations::report(
            &labels.suite,
            &scorer_version,
            output.semantic_reranking.clone(),
            output.semantic_obligation_yield.clone(),
            scorer_ablations.clone(),
            Vec::new(),
        );
        Some(scorer_ablations::write_default_artifact(&repo_root(), &report)?)
    } else {
        None
    };
    let vector_search = if let Some(outcome) = vector_partial {
        Some(outcome)
    } else if let Some(vector_request) = request.vector_search.as_ref() {
        let root = repo_root();
        if let Some(reason) = vector_budget_violation(
            vector_request,
            workspace_rows.len(),
            workspace_rows.len(),
            total_started,
        ) {
            reporter.event(
                "eval.vector.artifact",
                None,
                None,
                "writing budget-exceeded vector artifact",
            );
            Some(vector_search::partial_report(
                vector_search::VectorSearchPartialReportRun {
                    repo_root: &root,
                    suite: &labels.suite,
                    status: "budget-exceeded",
                    reason,
                    request: Some(vector_request),
                    symbolic_baseline: Some(&baseline_metrics),
                    vector_candidates: output.retrieval.vector_candidates.clone(),
                    started: total_started,
                },
            )?)
        } else {
            reporter.event("eval.vector.artifact", None, None, "writing vector validation artifact");
            Some(vector_search::report(vector_search::VectorSearchReportRun {
                repo_root: &root,
                suite: &labels.suite,
                labels: &labels,
                observation: &output,
                symbolic_baseline: &baseline_metrics,
                vector_metrics: &metrics,
                scorer_version: &scorer_version,
                k_values: &k_values,
                request: Some(vector_request),
                started: total_started,
            })?)
        }
    } else {
        None
    };

    Ok(EvalOutput {
        status: "ok".to_owned(),
        suite: labels.suite,
        scorer_version,
        metrics,
        search_dataset_artifact,
        scorer_ablation_artifact,
        vector_search_status: vector_search.as_ref().map(|outcome| outcome.status.clone()),
        vector_search_artifact: vector_search.as_ref().map(|outcome| outcome.artifact.clone()),
        scorer_ablations,
        vector_search_metrics: vector_search.and_then(|outcome| outcome.metrics),
        runs: Vec::new(),
    })
}

fn tracked_pairs(labels: &GoldLabels) -> Vec<SearchTrackedPair> {
    let mut pairs = labels
        .positives
        .iter()
        .chain(labels.hard_negatives.iter())
        .map(|pair| SearchTrackedPair {
            left: pair.left.clone(),
            right: pair.right.clone(),
        })
        .collect::<Vec<_>>();
    pairs.sort();
    pairs.dedup();
    pairs
}

fn scorer_ablation_variants(
    labels: &GoldLabels,
    base_observation: &SearchObservation,
    k_values: &[usize],
    index_load_ms: u128,
    reporter: &mut Reporter,
) -> Vec<ScorerAblationVariantReport> {
    let mut variants = Vec::new();
    for variant in SearchScoringVariant::all() {
        let started = Instant::now();
        let observation = rescore_observation(base_observation, variant);
        let retrieval_ms = started.elapsed().as_millis();
        reporter.event(
            "eval",
            None,
            None,
            format!(
                "scorer ablation {} observed {} pairs",
                variant.label(),
                observation.pairs.len()
            ),
        );
        let observed = ObservedRun {
            suite: labels.suite.clone(),
            pairs: observed_pairs(&observation),
            visible_groups: CountMetric {
                found: observation.visible_groups_found,
                total: observation.visible_groups_total,
            },
            probe_unavailable: CountMetric { found: 0, total: 0 },
            semantic_verification: SemanticVerificationStageMetrics {
                semantic_reranking: observation.semantic_reranking.clone(),
                obligation_yield: observation.semantic_obligation_yield.clone(),
                ..SemanticVerificationStageMetrics::default()
            },
            timings: TimingMetrics {
                index_load_ms,
                retrieval_ms,
                probe_ms: 0,
                total_ms: retrieval_ms,
            },
            peak_memory_bytes: perf::peak_rss_bytes(),
        };
        variants.push(ScorerAblationVariantReport {
            variant,
            status: "ok".to_owned(),
            semantic_reranking: observation.semantic_reranking.clone(),
            semantic_obligation_yield: observation.semantic_obligation_yield.clone(),
            metrics: Some(score_run(labels, &observed, k_values)),
            reason: None,
        });
    }
    variants
}

fn run_production_gate(request: EvalRequest, reporter: &mut Reporter) -> Result<EvalOutput> {
    let mut runs = Vec::new();
    for child in [EvalSuite::Default, EvalSuite::HardNegatives] {
        runs.push(run_child_suite(
            EvalRequest {
                suite: child,
                workspace: None,
                mathlib_workspace: None,
                manual_module: None,
                k_values: request.k_values.clone(),
                write_search_dataset: false,
                write_scorer_ablations: request.write_scorer_ablations,
                vector_search: request.vector_search.clone(),
            },
            false,
            reporter,
        ));
    }

    for child in [EvalSuite::ManualInternal, EvalSuite::ManualMathlib] {
        runs.push(run_child_suite(
            EvalRequest {
                suite: child,
                workspace: request.workspace.clone(),
                mathlib_workspace: request.mathlib_workspace.clone(),
                manual_module: request.manual_module.clone(),
                k_values: request.k_values.clone(),
                write_search_dataset: false,
                write_scorer_ablations: request.write_scorer_ablations,
                vector_search: request.vector_search.clone(),
            },
            true,
            reporter,
        ));
    }

    let completed_metrics = runs.iter().filter_map(|run| run.metrics.as_ref()).collect::<Vec<_>>();
    let metrics = aggregate_metrics("production-gate", &completed_metrics);
    let status = if runs.iter().any(|run| run.status == "failed") {
        "failed"
    } else if runs.iter().any(|run| run.status == "skipped") {
        "incomplete"
    } else {
        "ok"
    };
    let scorer_version = runs
        .iter()
        .find_map(|run| run.scorer_version.clone())
        .unwrap_or_else(|| {
            lean_dup_search::SearchScoringSummary::new(SearchScoringVariant::SymbolicOnly)
                .version
                .to_owned()
        });
    let scorer_ablations = if request.write_scorer_ablations {
        aggregate_scorer_ablations(&runs)
    } else {
        Vec::new()
    };
    let scorer_ablation_artifact = if request.write_scorer_ablations {
        let children = runs
            .iter()
            .map(|run| scorer_ablations::ScorerAblationChildReport {
                suite: run.suite.clone(),
                status: run.status.clone(),
                reason: run.reason.clone(),
                variants: run.scorer_ablations.clone(),
            })
            .collect();
        let semantic_reranking = completed_metrics
            .first()
            .map(|metrics| metrics.stage_metrics.semantic_verification.semantic_reranking.clone())
            .unwrap_or_default();
        let semantic_obligation_yield = metrics.stage_metrics.semantic_verification.obligation_yield.clone();
        let report = scorer_ablations::report(
            "production-gate",
            &scorer_version,
            semantic_reranking,
            semantic_obligation_yield,
            scorer_ablations.clone(),
            children,
        );
        Some(scorer_ablations::write_default_artifact(&repo_root(), &report)?)
    } else {
        None
    };
    let vector_search = if request.vector_search.is_some() {
        let children = runs
            .iter()
            .map(|run| {
                vector_search::child_report(
                    run.suite.clone(),
                    run.vector_search_status.clone(),
                    run.vector_search_artifact.clone(),
                    run.vector_search_metrics.clone(),
                    run.reason.clone(),
                )
            })
            .collect();
        Some(vector_search::aggregate(
            &repo_root(),
            "production-gate",
            &scorer_version,
            &metrics,
            children,
        )?)
    } else {
        None
    };

    Ok(EvalOutput {
        status: status.to_owned(),
        suite: "production-gate".to_owned(),
        scorer_version,
        metrics,
        search_dataset_artifact: None,
        scorer_ablation_artifact,
        vector_search_status: vector_search.as_ref().map(|outcome| outcome.status.clone()),
        vector_search_artifact: vector_search.as_ref().map(|outcome| outcome.artifact.clone()),
        scorer_ablations,
        vector_search_metrics: vector_search.and_then(|outcome| outcome.metrics),
        runs,
    })
}

fn run_child_suite(request: EvalRequest, manual: bool, reporter: &mut Reporter) -> EvaluationRunReport {
    if manual && !manual_workspace_exists(&request) {
        let reason = "manual suite workspace is unavailable".to_owned();
        let vector_search = unstarted_vector_partial(&request, "skipped", reason.clone());
        return EvaluationRunReport {
            suite: request.suite.as_str().to_owned(),
            status: "skipped".to_owned(),
            scorer_version: None,
            metrics: None,
            reason: Some(reason),
            vector_search_status: vector_search.as_ref().map(|outcome| outcome.status.clone()),
            vector_search_artifact: vector_search.as_ref().map(|outcome| outcome.artifact.clone()),
            manual,
            scorer_ablations: Vec::new(),
            vector_search_metrics: vector_search.and_then(|outcome| outcome.metrics),
        };
    }

    let suite = request.suite;
    let requested_vector_search = request.vector_search.clone();
    match run_single(request, reporter) {
        Ok(report) => EvaluationRunReport {
            suite: report.suite,
            status: report.status,
            scorer_version: Some(report.scorer_version),
            metrics: Some(report.metrics),
            reason: None,
            vector_search_status: report.vector_search_status,
            vector_search_artifact: report.vector_search_artifact,
            manual,
            scorer_ablations: report.scorer_ablations,
            vector_search_metrics: report.vector_search_metrics,
        },
        Err(error) => {
            let reason = error.to_string();
            let status = if manual && is_manual_prerequisite_error(&reason) {
                "skipped"
            } else {
                "failed"
            };
            let vector_search = unstarted_vector_partial(
                &EvalRequest {
                    suite,
                    workspace: None,
                    mathlib_workspace: None,
                    manual_module: None,
                    k_values: Vec::new(),
                    write_search_dataset: false,
                    write_scorer_ablations: false,
                    vector_search: requested_vector_search,
                },
                status,
                reason.clone(),
            );
            EvaluationRunReport {
                suite: suite.as_str().to_owned(),
                status: status.to_owned(),
                scorer_version: None,
                metrics: None,
                reason: Some(reason),
                vector_search_status: vector_search.as_ref().map(|outcome| outcome.status.clone()),
                vector_search_artifact: vector_search.as_ref().map(|outcome| outcome.artifact.clone()),
                manual,
                scorer_ablations: Vec::new(),
                vector_search_metrics: vector_search.and_then(|outcome| outcome.metrics),
            }
        }
    }
}

fn unstarted_vector_partial(
    request: &EvalRequest,
    status: &str,
    reason: String,
) -> Option<vector_search::VectorSearchArtifactOutcome> {
    let vector_request = request.vector_search.as_ref()?;
    let root = repo_root();
    let summary = budget_vector_summary(vector_request, 0, 0, reason.clone());
    vector_search::partial_report(vector_search::VectorSearchPartialReportRun {
        repo_root: &root,
        suite: request.suite.as_str(),
        status,
        reason,
        request: Some(vector_request),
        symbolic_baseline: None,
        vector_candidates: summary,
        started: Instant::now(),
    })
    .ok()
}

fn aggregate_scorer_ablations(runs: &[EvaluationRunReport]) -> Vec<ScorerAblationVariantReport> {
    SearchScoringVariant::all()
        .into_iter()
        .map(|variant| {
            let metrics = runs
                .iter()
                .flat_map(|run| &run.scorer_ablations)
                .filter(|ablation| ablation.variant == variant && ablation.status == "ok")
                .filter_map(|ablation| ablation.metrics.as_ref())
                .collect::<Vec<_>>();
            if metrics.is_empty() {
                ScorerAblationVariantReport {
                    variant,
                    status: "skipped".to_owned(),
                    semantic_reranking: lean_dup_search::SearchSemanticRerankingSummary::default(),
                    semantic_obligation_yield: Vec::new(),
                    metrics: None,
                    reason: Some("no completed child metrics".to_owned()),
                }
            } else {
                let aggregate = aggregate_metrics("production-gate", &metrics);
                ScorerAblationVariantReport {
                    variant,
                    status: "ok".to_owned(),
                    semantic_reranking: aggregate.stage_metrics.semantic_verification.semantic_reranking.clone(),
                    semantic_obligation_yield: aggregate.stage_metrics.semantic_verification.obligation_yield.clone(),
                    metrics: Some(aggregate),
                    reason: None,
                }
            }
        })
        .collect()
}

fn manual_workspace_exists(request: &EvalRequest) -> bool {
    request.workspace.as_ref().is_some_and(|path| path.exists())
}

fn manual_module(request: &EvalRequest) -> String {
    request.manual_module.clone().unwrap_or_else(|| "Workspace".to_owned())
}

fn is_manual_prerequisite_error(reason: &str) -> bool {
    reason.contains("missing compiled oleans")
        || (reason.contains("import_failed") && reason.contains("object file") && reason.contains("does not exist"))
        || reason.contains("workspace does not exist")
        || reason.contains("not a Lake workspace")
}

fn aggregate_metrics(suite: &str, runs: &[&EvaluationMetrics]) -> EvaluationMetrics {
    let k_values = {
        let mut values = runs
            .iter()
            .flat_map(|metrics| metrics.recall.iter().map(|recall| recall.k))
            .collect::<Vec<_>>();
        values.sort_unstable();
        values.dedup();
        values
    };
    let recall = k_values
        .into_iter()
        .map(|k| RecallAtK {
            k,
            found: runs
                .iter()
                .filter_map(|metrics| metrics.recall.iter().find(|recall| recall.k == k))
                .map(|recall| recall.found)
                .sum(),
            total: runs
                .iter()
                .filter_map(|metrics| metrics.recall.iter().find(|recall| recall.k == k))
                .map(|recall| recall.total)
                .sum(),
        })
        .collect();
    let stage_metrics = runs.iter().map(|metrics| &metrics.stage_metrics).collect::<Vec<_>>();

    EvaluationMetrics {
        suite: suite.to_owned(),
        recall,
        shown_queue_precision: sum_count(runs, |metrics| &metrics.shown_queue_precision),
        hard_negative_hits: sum_count(runs, |metrics| &metrics.hard_negative_hits),
        visible_groups: sum_count(runs, |metrics| &metrics.visible_groups),
        probe_unavailable: sum_count(runs, |metrics| &metrics.probe_unavailable),
        stage_metrics: crate::eval::stage_metrics::aggregate(suite, &stage_metrics),
        candidate_count: runs.iter().map(|metrics| metrics.candidate_count).sum(),
        timings: TimingMetrics {
            index_load_ms: runs.iter().map(|metrics| metrics.timings.index_load_ms).sum(),
            retrieval_ms: runs.iter().map(|metrics| metrics.timings.retrieval_ms).sum(),
            probe_ms: runs.iter().map(|metrics| metrics.timings.probe_ms).sum(),
            total_ms: runs.iter().map(|metrics| metrics.timings.total_ms).sum(),
        },
        peak_memory_bytes: runs.iter().filter_map(|metrics| metrics.peak_memory_bytes).max(),
    }
}

fn sum_count<'a>(
    runs: &[&'a EvaluationMetrics],
    metric: impl Fn(&'a EvaluationMetrics) -> &'a CountMetric,
) -> CountMetric {
    CountMetric {
        found: runs.iter().map(|run| metric(run).found).sum(),
        total: runs.iter().map(|run| metric(run).total).sum(),
    }
}

fn vector_budget_violation(
    request: &VectorSearchRequest,
    query_count: usize,
    corpus_count: usize,
    started: Instant,
) -> Option<String> {
    let total_declarations = query_count.saturating_add(corpus_count);
    if total_declarations > request.bounds.max_declarations {
        return Some(format!(
            "vector-validation-budget-exceeded:max-declarations:{total_declarations}>{}",
            request.bounds.max_declarations
        ));
    }
    if query_count > request.bounds.max_queries {
        return Some(format!(
            "vector-validation-budget-exceeded:max-queries:{query_count}>{}",
            request.bounds.max_queries
        ));
    }
    if started.elapsed().as_millis() > request.bounds.max_runtime_ms {
        return Some(format!(
            "vector-validation-budget-exceeded:max-runtime-ms:{}>{}",
            started.elapsed().as_millis(),
            request.bounds.max_runtime_ms
        ));
    }
    if let Some(peak_rss_bytes) = perf::peak_rss_bytes()
        && peak_rss_bytes > request.bounds.max_rss_bytes
    {
        return Some(format!(
            "vector-validation-budget-exceeded:max-rss-bytes:{peak_rss_bytes}>{}",
            request.bounds.max_rss_bytes
        ));
    }
    None
}

fn budget_vector_summary(
    request: &VectorSearchRequest,
    query_count: usize,
    corpus_count: usize,
    reason: String,
) -> SearchVectorCandidateSummary {
    let eligibility = |total| SearchVectorEligibilitySummary {
        policy_id: request.eligibility_policy.id().to_owned(),
        policy_version: "lean-dup.vector-candidate.v1",
        total,
        eligible: 0,
        skipped_by_reason: Default::default(),
    };
    SearchVectorCandidateSummary {
        status: SearchVectorCandidateStatus::Skipped,
        reason: Some(reason),
        model_profile_id: request.profile_id.clone(),
        input_format_id: request.input_format.id().to_owned(),
        input_format_version: request.input_format.version().to_owned(),
        acquisition_policy: request.acquisition_policy,
        document_policy_id: request.document_policy.id().to_owned(),
        document_policy_version: "lean-dup.embedding-document.v1".to_owned(),
        query_declaration_count: query_count,
        corpus_declaration_count: corpus_count,
        query_eligibility: eligibility(query_count),
        corpus_eligibility: eligibility(corpus_count),
        top_k: VECTOR_CANDIDATE_TOP_K,
        eligible_corpus_size: corpus_count,
        top_k_saturated: VECTOR_CANDIDATE_TOP_K >= corpus_count,
        ..SearchVectorCandidateSummary::default()
    }
}

fn suite_k_values(suite: EvalSuite, requested: &[usize]) -> Vec<usize> {
    let mut values = requested.to_vec();
    if suite == EvalSuite::Default && !values.contains(&10) {
        values.push(10);
    }
    values
}

fn vector_fixture_declarations() -> Vec<HydratedDeclaration> {
    let mut declarations = vec![
        fixture_declaration(
            "VectorFixture.vector_only_query",
            "theorem",
            "semantic source has additive identity on the right",
            "vector-only-query",
        ),
        fixture_declaration(
            "VectorFixture.vector_only_match",
            "theorem",
            "semantic target has additive identity on the right",
            "vector-only-match",
        ),
        fixture_declaration(
            "VectorFixture.symbolic_only_query",
            "theorem",
            "symbolic duplicate proposition",
            "symbolic-only-shared",
        ),
        fixture_declaration(
            "VectorFixture.symbolic_only_document",
            "theorem",
            "symbolic duplicate proposition",
            "symbolic-only-shared",
        ),
        fixture_declaration(
            "LexicalTrap.height",
            "theorem",
            "height statement over arithmetic objects",
            "lexical-height-left",
        ),
        fixture_declaration(
            "LexicalTrap.height_not_duplicate",
            "theorem",
            "height statement over topological objects",
            "lexical-height-right",
        ),
    ];
    declarations.extend((0..66).map(|index| {
        fixture_declaration(
            &format!("VectorFixture.filler_{index:02}"),
            "theorem",
            &format!("filler theorem statement {index}"),
            &format!("filler-{index:02}"),
        )
    }));
    declarations.extend([
        fixture_declaration(
            "VectorFixture.generated_noise.rec",
            "theorem",
            "generated declaration should not enter the default vector corpus",
            "skip-generated",
        )
        .with_status_flag("generated"),
        fixture_declaration(
            "VectorFixture.private_noise",
            "theorem",
            "private declaration should not enter the default vector corpus",
            "skip-private",
        )
        .with_visibility("private"),
        fixture_declaration(
            "Synthetic.fixture_noise",
            "theorem",
            "synthetic declaration should not enter the default vector corpus",
            "skip-synthetic",
        ),
        fixture_declaration(
            "VectorFixture.low_signal_noise",
            "theorem",
            "low signal equality should not enter the default vector corpus",
            "skip-low-signal",
        )
        .with_low_signal("broad_head:Eq"),
        fixture_declaration(
            "VectorFixture.missing_statement_noise",
            "theorem",
            "",
            "skip-missing-statement",
        ),
        fixture_declaration(
            "VectorFixture.instNoise",
            "instance",
            "non-actionable instance should not enter the default vector corpus",
            "skip-not-actionable",
        ),
        fixture_declaration(
            "VectorFixture.structure_noise",
            "structure",
            "unsupported declaration kind should not enter the default vector corpus",
            "skip-unsupported-kind",
        ),
    ]);
    declarations
}

fn fixture_declaration(name: &str, kind: &str, statement: &str, statement_fingerprint: &str) -> HydratedDeclaration {
    HydratedDeclaration {
        handle: DeclarationHandle::for_test(name),
        declaration_id: format!("fixture:{name}"),
        origin: "workspace".to_owned(),
        module: name.split('.').next().unwrap_or("VectorFixture").to_owned(),
        qualified_name: name.to_owned(),
        display_name: name.rsplit('.').next().unwrap_or(name).to_owned(),
        kind: kind.to_owned(),
        visibility: "public".to_owned(),
        modifiers: Vec::new(),
        source_span: None,
        statement_text: statement.to_owned(),
        docstring_text: None,
        definition_body_summary: None,
        status_flags: Vec::new(),
        feature_version: "fixture".to_owned(),
        fingerprints: Fingerprints {
            statement: statement_fingerprint.to_owned(),
            safe_binder_permutation: String::new(),
            connective_shape: String::new(),
            conclusion_shape: String::new(),
        },
        role_features: Vec::new(),
        binder_count: 0,
        low_signal_markers: Vec::new(),
    }
}

trait VectorFixtureDeclarationExt {
    fn with_status_flag(self, flag: &str) -> Self;
    fn with_visibility(self, visibility: &str) -> Self;
    fn with_low_signal(self, marker: &str) -> Self;
}

impl VectorFixtureDeclarationExt for HydratedDeclaration {
    fn with_status_flag(mut self, flag: &str) -> Self {
        self.status_flags.push(flag.to_owned());
        self
    }

    fn with_visibility(mut self, visibility: &str) -> Self {
        self.visibility = visibility.to_owned();
        self
    }

    fn with_low_signal(mut self, marker: &str) -> Self {
        self.low_signal_markers.push(marker.to_owned());
        self
    }
}

fn suite_definition(request: &EvalRequest) -> SuiteDefinition {
    let repo = repo_root();
    match request.suite {
        EvalSuite::Default => SuiteDefinition {
            suite: request.suite,
            workspace: request
                .workspace
                .clone()
                .unwrap_or_else(|| repo.join("tests/fixtures/tiny")),
            module_root: "Tiny".to_owned(),
            origin: "workspace".to_owned(),
            external: Some(ExternalSuiteIndex {
                workspace: repo.join("tests/fixtures/external"),
                module_root: "External".to_owned(),
                label: "eval-default-external".to_owned(),
                origin: "external:fixture".to_owned(),
                require_oleans: false,
            }),
            mathlib_source_override: None,
            build_before_index: true,
            require_oleans: false,
        },
        EvalSuite::HardNegatives => SuiteDefinition {
            suite: request.suite,
            workspace: request
                .workspace
                .clone()
                .unwrap_or_else(|| repo.join("tests/fixtures/tiny")),
            module_root: "Tiny".to_owned(),
            origin: "workspace".to_owned(),
            external: Some(ExternalSuiteIndex {
                workspace: repo.join("tests/fixtures/external"),
                module_root: "External".to_owned(),
                label: "eval-hard-negatives-external".to_owned(),
                origin: "external:fixture".to_owned(),
                require_oleans: false,
            }),
            mathlib_source_override: None,
            build_before_index: true,
            require_oleans: false,
        },
        EvalSuite::VectorFixture => unreachable!("vector-fixture uses in-memory command-level fixture declarations"),
        EvalSuite::ManualInternal => SuiteDefinition {
            suite: request.suite,
            workspace: request.workspace.clone().unwrap_or_default(),
            module_root: manual_module(request),
            origin: "workspace".to_owned(),
            external: None,
            mathlib_source_override: None,
            build_before_index: false,
            require_oleans: true,
        },
        EvalSuite::ManualMathlib => SuiteDefinition {
            suite: request.suite,
            workspace: request.workspace.clone().unwrap_or_default(),
            module_root: manual_module(request),
            origin: "workspace".to_owned(),
            external: Some(ExternalSuiteIndex {
                workspace: request.workspace.clone().unwrap_or_default(),
                module_root: "Mathlib".to_owned(),
                label: "eval-manual-mathlib".to_owned(),
                origin: "mathlib".to_owned(),
                require_oleans: true,
            }),
            mathlib_source_override: request.mathlib_workspace.clone(),
            build_before_index: false,
            require_oleans: true,
        },
        EvalSuite::ProductionGate => unreachable!("production-gate is expanded before suite definition"),
    }
}

fn cache_root_for(definition: &SuiteDefinition, reporter: &mut Reporter) -> Result<PathBuf> {
    let workspace = resolve(
        WorkspaceRequest {
            requested_root: definition.workspace.clone(),
            module_root: Some(definition.module_root.clone()),
        },
        reporter,
    )?;
    Ok(lean_dup_index::resolve_cache(&workspace)?.root)
}

fn build_or_load_project_mathlib_index(
    definition: &SuiteDefinition,
    external: &ExternalSuiteIndex,
    cache_root: &Path,
    reporter: &mut Reporter,
) -> Result<OpenedIndex> {
    let mathlib = resolve_project_mathlib(
        definition.workspace.clone(),
        definition.mathlib_source_override.clone(),
        reporter,
    )?;
    let execution_root = mathlib.execution_root();
    let store = IndexStore::new(cache_root.to_path_buf());
    store.build_or_reuse(
        IndexBuildRequest {
            workspace: mathlib.source,
            execution_root: Some(execution_root),
            label: external.label.clone(),
            module_root: external.module_root.clone(),
            origin: external.origin.clone(),
            include_private: true,
            include_generated: false,
            require_oleans: external.require_oleans,
            force: false,
            kind: IndexBuildKind::ProjectMathlib,
        },
        &WorkerClient::for_indexing(),
        reporter,
    )?;
    Ok(store.resolve(IndexReference::Label(external.label.clone()))?)
}

fn build_or_load_index(
    request: SuiteIndexRequest<'_>,
    cache_root: &Path,
    reporter: &mut Reporter,
) -> Result<OpenedIndex> {
    if request.build_before_index {
        lake_build(request.workspace_root)?;
    }
    let workspace = resolve(
        WorkspaceRequest {
            requested_root: request.workspace_root.to_path_buf(),
            module_root: Some(request.module_root.to_owned()),
        },
        reporter,
    )?;
    let store = IndexStore::new(cache_root.to_path_buf());
    store.build_or_reuse(
        IndexBuildRequest {
            workspace,
            execution_root: None,
            label: request.label.to_owned(),
            module_root: request.module_root.to_owned(),
            origin: request.origin.to_owned(),
            include_private: true,
            include_generated: false,
            require_oleans: request.require_oleans,
            force: false,
            kind: request.kind,
        },
        &WorkerClient::for_indexing(),
        reporter,
    )?;
    Ok(store.resolve(IndexReference::Label(request.label.to_owned()))?)
}

fn lake_build(workspace_root: &Path) -> Result<()> {
    let output = Command::new("lake")
        .arg("build")
        .current_dir(workspace_root)
        .output()
        .map_err(|source| Error::Io {
            message: "could not run lake build",
            path: workspace_root.to_path_buf(),
            source,
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Error::Eval {
            message: format!(
                "lake build failed in {}: {}{}",
                workspace_root.display(),
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout)
            ),
        })
    }
}

fn observed_pairs(output: &SearchObservation) -> Vec<ObservedPair> {
    output
        .pairs
        .iter()
        .map(|pair| ObservedPair {
            pair: GoldPair::new(pair.left.clone(), pair.right.clone()),
            generated: pair.generated,
            symbolic_generated: pair.symbolic_generated,
            vector_generated: pair.vector_generated,
            merged_generated: pair.merged_generated,
            ranked: pair.ranked,
            generation_policy: pair.generation_policy.clone(),
            rank: pair.rank,
            shown: pair.shown,
            origin: pair.origin.clone(),
            feature_families: pair.feature_families.clone(),
            survived_shown_filter: pair.survived_shown_filter,
        })
        .collect()
}

fn enforce_suite_gates(definition: &SuiteDefinition, labels: &GoldLabels, metrics: &EvaluationMetrics) -> Result<()> {
    if matches!(definition.suite, EvalSuite::Default | EvalSuite::HardNegatives)
        && metrics.hard_negative_hits.found != 0
    {
        return Err(Error::Eval {
            message: format!(
                "{} suite hard-negative gate failed: {}/{} appeared in the shown queue",
                definition.suite.as_str(),
                metrics.hard_negative_hits.found,
                metrics.hard_negative_hits.total
            ),
        });
    }
    if definition.suite != EvalSuite::Default {
        return Ok(());
    }
    let Some(recall_10) = metrics.recall.iter().find(|recall| recall.k == 10) else {
        return Err(Error::Eval {
            message: "default suite requires recall@10".to_owned(),
        });
    };
    if recall_10.found != labels.positives.len() {
        return Err(Error::Eval {
            message: format!(
                "default suite recall@10 gate failed: found {}/{} positives",
                recall_10.found,
                labels.positives.len()
            ),
        });
    }
    Ok(())
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate lives under repo/crates/<component>")
        .to_path_buf()
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{EvalRequest, aggregate_metrics, run, run_child_suite};
    use crate::EvalSuite;
    use crate::eval::scoring::{CountMetric, EvaluationMetrics, RecallAtK, TimingMetrics};
    use crate::eval::stage_metrics::{SearchStageMetrics, SemanticVerificationStageMetrics};
    use lean_dup_diagnostics::progress::Reporter;

    #[test]
    fn default_suite_computes_metrics_and_enforces_gates() {
        let cache = TempDir::new().unwrap();
        let previous = std::env::var_os("LEAN_DUP_CACHE_DIR");
        unsafe {
            std::env::set_var("LEAN_DUP_CACHE_DIR", cache.path());
        }
        let result = run(
            EvalRequest {
                suite: EvalSuite::Default,
                workspace: None,
                mathlib_workspace: None,
                manual_module: None,
                k_values: vec![1, 5, 10],
                write_search_dataset: false,
                write_scorer_ablations: false,
                vector_search: None,
            },
            &mut Reporter::new(false, false),
        );
        restore_env(previous);

        let report = result.unwrap();
        assert_eq!(report.status, "ok");
        assert_eq!(report.metrics.suite, "default");
        assert!(
            report
                .metrics
                .recall
                .iter()
                .any(|recall| recall.k == 10 && recall.found == recall.total)
        );
        assert_eq!(report.metrics.hard_negative_hits.found, 0);
        assert!(report.metrics.candidate_count > 0);
    }

    #[test]
    fn aggregate_metrics_sums_counts_and_keeps_peak_memory() {
        let first = metrics("a", 1, 2, Some(10));
        let second = metrics("b", 3, 4, Some(30));

        let aggregate = aggregate_metrics("production-gate", &[&first, &second]);

        assert_eq!(aggregate.suite, "production-gate");
        assert_eq!(aggregate.recall[0].found, 4);
        assert_eq!(aggregate.recall[0].total, 6);
        assert_eq!(aggregate.shown_queue_precision.found, 4);
        assert_eq!(aggregate.shown_queue_precision.total, 6);
        assert_eq!(aggregate.hard_negative_hits.found, 0);
        assert_eq!(aggregate.hard_negative_hits.total, 6);
        assert_eq!(aggregate.visible_groups.found, 4);
        assert_eq!(aggregate.visible_groups.total, 6);
        assert_eq!(aggregate.probe_unavailable.found, 4);
        assert_eq!(aggregate.probe_unavailable.total, 6);
        assert_eq!(aggregate.stage_metrics.candidate_generation_recall.found, 4);
        assert_eq!(aggregate.stage_metrics.candidate_generation_recall.total, 6);
        assert_eq!(aggregate.stage_metrics.semantic_verification.planned, 4);
        assert_eq!(aggregate.stage_metrics.semantic_verification.cached, 6);
        assert_eq!(aggregate.peak_memory_bytes, Some(30));
    }

    #[test]
    fn manual_child_suite_with_missing_workspace_is_skipped() {
        let missing = TempDir::new().unwrap().path().join("missing-manual");
        let report = run_child_suite(
            EvalRequest {
                suite: EvalSuite::ManualInternal,
                workspace: Some(missing),
                mathlib_workspace: None,
                manual_module: None,
                k_values: vec![1, 5, 10],
                write_search_dataset: false,
                write_scorer_ablations: false,
                vector_search: None,
            },
            true,
            &mut Reporter::new(false, false),
        );

        assert_eq!(report.suite, "manual-internal");
        assert_eq!(report.status, "skipped");
        assert!(report.manual);
    }

    #[test]
    fn missing_oleans_are_manual_prerequisites_not_gate_failures() {
        assert!(super::is_manual_prerequisite_error(
            "index error: missing compiled oleans for index"
        ));
        assert!(super::is_manual_prerequisite_error(
            "worker returned a fatal diagnostic: import_failed fatal: object file '/tmp/Workspace.olean' does not exist"
        ));
        assert!(!super::is_manual_prerequisite_error(
            "evaluation error: default suite recall@10 gate failed"
        ));
    }

    #[test]
    #[ignore = "manual slow suite over private corpus"]
    fn manual_internal_suite_runs_when_requested() {
        let report = run(
            EvalRequest {
                suite: EvalSuite::ManualInternal,
                workspace: None,
                mathlib_workspace: None,
                manual_module: None,
                k_values: vec![1, 5, 10],
                write_search_dataset: false,
                write_scorer_ablations: false,
                vector_search: None,
            },
            &mut Reporter::new(false, true),
        )
        .unwrap();
        assert_eq!(report.status, "ok");
    }

    #[test]
    #[ignore = "manual slow suite over private corpus and mathlib indexes"]
    fn manual_mathlib_suite_runs_when_requested() {
        let report = run(
            EvalRequest {
                suite: EvalSuite::ManualMathlib,
                workspace: None,
                mathlib_workspace: None,
                manual_module: None,
                k_values: vec![1, 5, 10],
                write_search_dataset: false,
                write_scorer_ablations: false,
                vector_search: None,
            },
            &mut Reporter::new(false, true),
        )
        .unwrap();
        assert_eq!(report.status, "ok");
    }

    fn restore_env(previous: Option<std::ffi::OsString>) {
        unsafe {
            match previous {
                Some(value) => std::env::set_var("LEAN_DUP_CACHE_DIR", value),
                None => std::env::remove_var("LEAN_DUP_CACHE_DIR"),
            }
        }
    }

    fn metrics(suite: &str, found: usize, total: usize, peak_memory_bytes: Option<u64>) -> EvaluationMetrics {
        EvaluationMetrics {
            suite: suite.to_owned(),
            recall: vec![RecallAtK { k: 10, found, total }],
            shown_queue_precision: CountMetric { found, total },
            hard_negative_hits: CountMetric { found: 0, total },
            visible_groups: CountMetric { found, total },
            probe_unavailable: CountMetric { found, total },
            stage_metrics: SearchStageMetrics {
                candidate_generation_recall: CountMetric { found, total },
                candidate_stage_recall: Default::default(),
                top_k_recall_before_final_ranking: vec![RecallAtK { k: 10, found, total }],
                ranked_recall: vec![RecallAtK { k: 10, found, total }],
                visible_queue_precision: CountMetric { found, total },
                hard_negative_survival: Default::default(),
                hard_negative_stage_survival: Default::default(),
                candidate_count_by_origin: Default::default(),
                candidate_count_by_feature_family: Default::default(),
                generated_candidate_count_by_policy: Default::default(),
                generated_candidate_count_by_feature_family: Default::default(),
                hard_negative_generated_by_feature_family: Default::default(),
                semantic_verification: SemanticVerificationStageMetrics {
                    semantic_reranking: lean_dup_search::SearchSemanticRerankingSummary::default(),
                    planned: found,
                    cached: total,
                    worker: 0,
                    unavailable: found,
                    obligation_yield: Vec::new(),
                },
            },
            candidate_count: total,
            timings: TimingMetrics {
                index_load_ms: total as u128,
                retrieval_ms: total as u128,
                probe_ms: total as u128,
                total_ms: total as u128,
            },
            peak_memory_bytes,
        }
    }
}
