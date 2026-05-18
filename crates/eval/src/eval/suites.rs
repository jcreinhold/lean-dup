use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use serde::Serialize;

use crate::EvalSuite;
use crate::eval::labels::{GoldLabels, load_builtin};
use crate::eval::memory;
use crate::eval::scoring::{
    CountMetric, EvaluationMetrics, GoldPair, ObservedPair, ObservedRun, RecallAtK, TimingMetrics, score_run,
};
use crate::eval::stage_metrics::SemanticVerificationStageMetrics;
use lean_dup_diagnostics::progress::Reporter;
use lean_dup_index::{IndexBuildKind, IndexBuildRequest, IndexReference, IndexStore, OpenedIndex};
use lean_dup_project::workspace::{WorkspaceRequest, resolve};
use lean_dup_search::observation::{SearchObservation, SearchObservationRequest, observe_search};
use lean_dup_worker::WorkerClient;

use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct EvalRequest {
    pub suite: EvalSuite,
    pub workspace: Option<PathBuf>,
    pub mathlib_workspace: Option<PathBuf>,
    pub k_values: Vec<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvaluationReport {
    pub status: String,
    pub suite: String,
    pub metrics: EvaluationMetrics,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub runs: Vec<EvaluationRunReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvaluationRunReport {
    pub suite: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<EvaluationMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub manual: bool,
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

pub fn run(request: EvalRequest, reporter: &mut Reporter) -> Result<EvaluationReport> {
    if request.suite == EvalSuite::ProductionGate {
        return run_production_gate(request, reporter);
    }
    run_single(request, reporter)
}

fn run_single(request: EvalRequest, reporter: &mut Reporter) -> Result<EvaluationReport> {
    let total_started = Instant::now();
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
            Some(external) if definition.suite == EvalSuite::KanproofsMathlib => Some(
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

    let retrieval_started = Instant::now();
    let output = match &external {
        Some(external) => observe_search(SearchObservationRequest {
            workspace: &workspace_rows,
            comparison_indexes: std::slice::from_ref(external),
        })?,
        None => observe_search(SearchObservationRequest {
            workspace: &workspace_rows,
            comparison_indexes: &[],
        })?,
    };
    let retrieval_ms = retrieval_started.elapsed().as_millis();

    let observed = ObservedRun {
        suite: labels.suite.clone(),
        pairs: observed_pairs(&output),
        visible_groups: CountMetric {
            found: output.visible_groups_found,
            total: output.visible_groups_total,
        },
        probe_unavailable: CountMetric { found: 0, total: 0 },
        semantic_verification: SemanticVerificationStageMetrics::default(),
        timings: TimingMetrics {
            index_load_ms,
            retrieval_ms,
            probe_ms: 0,
            total_ms: total_started.elapsed().as_millis(),
        },
        peak_memory_bytes: memory::peak_rss_bytes(),
    };
    let metrics = score_run(&labels, &observed, &k_values);
    enforce_suite_gates(&definition, &labels, &metrics)?;

    Ok(EvaluationReport {
        status: "ok".to_owned(),
        suite: labels.suite,
        metrics,
        runs: Vec::new(),
    })
}

fn run_production_gate(request: EvalRequest, reporter: &mut Reporter) -> Result<EvaluationReport> {
    let mut runs = Vec::new();
    for child in [EvalSuite::Default, EvalSuite::HardNegatives] {
        runs.push(run_child_suite(
            EvalRequest {
                suite: child,
                workspace: None,
                mathlib_workspace: None,
                k_values: request.k_values.clone(),
            },
            false,
            reporter,
        ));
    }

    for child in [EvalSuite::KanproofsInternal, EvalSuite::KanproofsMathlib] {
        runs.push(run_child_suite(
            EvalRequest {
                suite: child,
                workspace: request.workspace.clone(),
                mathlib_workspace: request.mathlib_workspace.clone(),
                k_values: request.k_values.clone(),
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

    Ok(EvaluationReport {
        status: status.to_owned(),
        suite: "production-gate".to_owned(),
        metrics,
        runs,
    })
}

fn run_child_suite(request: EvalRequest, manual: bool, reporter: &mut Reporter) -> EvaluationRunReport {
    if manual && !manual_workspace_exists(&request) {
        return EvaluationRunReport {
            suite: request.suite.as_str().to_owned(),
            status: "skipped".to_owned(),
            metrics: None,
            reason: Some("manual suite workspace is unavailable".to_owned()),
            manual,
        };
    }

    let suite = request.suite;
    match run_single(request, reporter) {
        Ok(report) => EvaluationRunReport {
            suite: report.suite,
            status: report.status,
            metrics: Some(report.metrics),
            reason: None,
            manual,
        },
        Err(error) => {
            let reason = error.to_string();
            let status = if manual && is_manual_prerequisite_error(&reason) {
                "skipped"
            } else {
                "failed"
            };
            EvaluationRunReport {
                suite: suite.as_str().to_owned(),
                status: status.to_owned(),
                metrics: None,
                reason: Some(reason),
                manual,
            }
        }
    }
}

fn manual_workspace_exists(request: &EvalRequest) -> bool {
    request
        .workspace
        .clone()
        .unwrap_or_else(|| PathBuf::from("/Users/jcreinhold/Code/kan-proofs"))
        .exists()
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

fn suite_k_values(suite: EvalSuite, requested: &[usize]) -> Vec<usize> {
    let mut values = requested.to_vec();
    if suite == EvalSuite::Default && !values.contains(&10) {
        values.push(10);
    }
    values
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
        EvalSuite::KanproofsInternal => SuiteDefinition {
            suite: request.suite,
            workspace: request
                .workspace
                .clone()
                .unwrap_or_else(|| PathBuf::from("/Users/jcreinhold/Code/kan-proofs")),
            module_root: "KanProofs".to_owned(),
            origin: "workspace".to_owned(),
            external: None,
            mathlib_source_override: None,
            build_before_index: false,
            require_oleans: true,
        },
        EvalSuite::KanproofsMathlib => SuiteDefinition {
            suite: request.suite,
            workspace: request
                .workspace
                .clone()
                .unwrap_or_else(|| PathBuf::from("/Users/jcreinhold/Code/kan-proofs")),
            module_root: "KanProofs".to_owned(),
            origin: "workspace".to_owned(),
            external: Some(ExternalSuiteIndex {
                workspace: request
                    .workspace
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("/Users/jcreinhold/Code/kan-proofs")),
                module_root: "Mathlib".to_owned(),
                label: "eval-kanproofs-mathlib".to_owned(),
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
    let mathlib = lean_dup_project::mathlib::resolve_project(
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
                k_values: vec![1, 5, 10],
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
        let missing = TempDir::new().unwrap().path().join("missing-kanproofs");
        let report = run_child_suite(
            EvalRequest {
                suite: EvalSuite::KanproofsInternal,
                workspace: Some(missing),
                mathlib_workspace: None,
                k_values: vec![1, 5, 10],
            },
            true,
            &mut Reporter::new(false, false),
        );

        assert_eq!(report.suite, "kanproofs-internal");
        assert_eq!(report.status, "skipped");
        assert!(report.manual);
    }

    #[test]
    fn missing_oleans_are_manual_prerequisites_not_gate_failures() {
        assert!(super::is_manual_prerequisite_error(
            "index error: missing compiled oleans for index"
        ));
        assert!(super::is_manual_prerequisite_error(
            "worker returned a fatal diagnostic: import_failed fatal: object file '/tmp/KanProofs.olean' does not exist"
        ));
        assert!(!super::is_manual_prerequisite_error(
            "evaluation error: default suite recall@10 gate failed"
        ));
    }

    #[test]
    #[ignore = "manual slow suite over KanProofs workspace"]
    fn kanproofs_internal_suite_runs_when_requested() {
        let report = run(
            EvalRequest {
                suite: EvalSuite::KanproofsInternal,
                workspace: None,
                mathlib_workspace: None,
                k_values: vec![1, 5, 10],
            },
            &mut Reporter::new(false, true),
        )
        .unwrap();
        assert_eq!(report.status, "ok");
    }

    #[test]
    #[ignore = "manual slow suite over KanProofs and mathlib indexes"]
    fn kanproofs_mathlib_suite_runs_when_requested() {
        let report = run(
            EvalRequest {
                suite: EvalSuite::KanproofsMathlib,
                workspace: None,
                mathlib_workspace: None,
                k_values: vec![1, 5, 10],
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
                top_k_recall_before_final_ranking: vec![RecallAtK { k: 10, found, total }],
                ranked_recall: vec![RecallAtK { k: 10, found, total }],
                visible_queue_precision: CountMetric { found, total },
                hard_negative_survival: Default::default(),
                candidate_count_by_origin: Default::default(),
                candidate_count_by_feature_family: Default::default(),
                semantic_verification: SemanticVerificationStageMetrics {
                    planned: found,
                    cached: total,
                    worker: 0,
                    unavailable: found,
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
