use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use lean_dup_diagnostics::perf;
use lean_dup_diagnostics::progress::Reporter;
use lean_dup_eval::{
    CountMetric, EvalSuite, EvaluationMetrics, GoldLabels, ObservedCandidateSource, ObservedPair, ObservedRun,
    TimingMetrics, load_builtin, parse_json, score_run,
};
use lean_dup_index::{DeclarationHandle, IndexBuildKind, IndexBuildRequest, IndexReference, IndexStore, OpenedIndex};
use lean_dup_project::{WorkspaceRequest, resolve, resolve_project_mathlib};
use lean_dup_search::{
    SearchObservation, SearchObservationRequest, SearchScoringVariant, SearchTrackedPair, observe_search,
};
use lean_dup_worker::{Fingerprints, RoleFeature, WorkerClient};

use crate::artifacts::{
    VectorScorerVariantReport, VectorSearchChildReport, VectorSearchReport, VectorValidationCostSummary, pair_reports,
    report_path, vector_stage_metrics, write,
};
use crate::candidates::{self, VectorCandidateStatus};
use crate::scoring::{
    VECTOR_FEATURE_VERSION, VectorScorerVariant, merge_pairs, observed_run, rank_pairs, top_k_saturation,
};
use crate::{
    Error, Result, VECTOR_SEARCH_SCHEMA_VERSION, VectorValidationOutcome, VectorValidationRequest,
    VectorValidationStatus,
};

struct SuiteDefinition {
    labels: GoldLabels,
    workspace: Vec<lean_dup_index::HydratedDeclaration>,
    comparison: Vec<lean_dup_index::HydratedDeclaration>,
    index_load_ms: u128,
}

pub(crate) fn run(request: VectorValidationRequest, reporter: &mut Reporter) -> Result<VectorValidationOutcome> {
    if request.suite == "production-gate" {
        return run_production_gate(request, reporter);
    }
    let started = Instant::now();
    let suite = load_suite(&request, reporter)?;
    run_loaded_suite(request, suite, started, reporter)
}

fn run_loaded_suite(
    request: VectorValidationRequest,
    suite: SuiteDefinition,
    started: Instant,
    reporter: &mut Reporter,
) -> Result<VectorValidationOutcome> {
    let tracked_pairs = tracked_pairs(&suite.labels);
    let retrieval_started = Instant::now();
    let symbolic = observe_search(SearchObservationRequest {
        workspace: &suite.workspace,
        comparison_indexes: &[],
        tracked_pairs: &tracked_pairs,
        scoring_variant: SearchScoringVariant::SymbolicOnly,
    })?;
    let base_retrieval_ms = retrieval_started.elapsed().as_millis();
    let baseline_observed = observed_from_symbolic(
        &suite.labels.suite,
        &symbolic,
        TimingMetrics {
            index_load_ms: suite.index_load_ms,
            retrieval_ms: base_retrieval_ms,
            probe_ms: 0,
            total_ms: started.elapsed().as_millis(),
        },
    );
    let baseline_metrics = score_run(&suite.labels, &baseline_observed, &suite_k_values(&request));

    if let Some(reason) = budget_violation(&request, suite.workspace.len(), comparison_count(&suite), started) {
        return write_partial(request, &suite, &baseline_metrics, reason, started);
    }

    reporter.event(
        "vector-search.validation",
        None,
        None,
        "running hidden vector candidate generation",
    );
    let vector = candidates::generate(&request, &suite.workspace, &suite.comparison, reporter)?;
    let merged_pairs = merge_pairs(&symbolic, vector.candidates);
    let mut variant_reports = Vec::new();
    let mut primary_metrics = None;
    let mut primary_rows = Vec::new();
    let mut primary_stage = Default::default();
    for variant in VectorScorerVariant::all() {
        let variant_started = Instant::now();
        let (ranked, visible_groups) = rank_pairs(&merged_pairs, variant);
        let observed = observed_run(
            &suite.labels.suite,
            &ranked,
            &visible_groups,
            TimingMetrics {
                index_load_ms: suite.index_load_ms,
                retrieval_ms: base_retrieval_ms.saturating_add(vector.summary.total_ms),
                probe_ms: 0,
                total_ms: variant_started.elapsed().as_millis(),
            },
            perf::peak_rss_bytes(),
        );
        let metrics = score_run(&suite.labels, &observed, &suite_k_values(&request));
        let rows = pair_reports(&suite.labels, &ranked);
        let stage = vector_stage_metrics(&suite.labels, &rows, top_k_saturation(&vector.summary));
        if variant == VectorScorerVariant::SymbolicPlusVector {
            primary_metrics = Some(metrics.clone());
            primary_rows = rows;
            primary_stage = stage.clone();
        }
        variant_reports.push(VectorScorerVariantReport {
            scorer_variant_id: variant.id().to_owned(),
            vector_feature_version: VECTOR_FEATURE_VERSION.to_owned(),
            metrics,
            vector_stage_metrics: stage,
        });
    }

    let artifact = report_path(&suite.labels.suite);
    let report = VectorSearchReport {
        schema_version: VECTOR_SEARCH_SCHEMA_VERSION,
        suite: suite.labels.suite.clone(),
        status: status_label(vector.summary.status).to_owned(),
        reason: vector.summary.reason.clone(),
        vector_candidates: vector.summary.clone(),
        symbolic_baseline: baseline_metrics,
        vector_search: primary_metrics.clone(),
        vector_stage_metrics: primary_stage,
        scorer_variants: variant_reports,
        pairs: primary_rows,
        validation_bounds: request.bounds,
        validation_cost: cost_summary(&vector.summary, perf::peak_rss_bytes(), Some(artifact.clone())),
        children: Vec::new(),
    };
    let root = request.artifact_root();
    let artifact = write(&root, artifact, &report)?;
    Ok(VectorValidationOutcome {
        schema_version: VECTOR_SEARCH_SCHEMA_VERSION,
        status: if vector.summary.status == VectorCandidateStatus::Ok {
            VectorValidationStatus::Ok
        } else {
            VectorValidationStatus::Skipped
        },
        suite: suite.labels.suite,
        artifact: Some(artifact),
        reason: vector.summary.reason,
    })
}

fn run_production_gate(request: VectorValidationRequest, reporter: &mut Reporter) -> Result<VectorValidationOutcome> {
    let mut children = Vec::new();
    for suite in ["default", "hard-negatives", "manual-internal", "manual-mathlib"] {
        let child_request = request.clone().with_suite(suite);
        let child = match run(child_request, reporter) {
            Ok(outcome) => VectorSearchChildReport {
                suite: suite.to_owned(),
                status: status_name(outcome.status).to_owned(),
                reason: outcome.reason,
                artifact: outcome.artifact,
                metrics: None,
            },
            Err(error) if suite.starts_with("manual-") => VectorSearchChildReport {
                suite: suite.to_owned(),
                status: "skipped".to_owned(),
                reason: Some(error.to_string()),
                artifact: None,
                metrics: None,
            },
            Err(error) => return Err(error),
        };
        children.push(child);
    }
    let status = if children.iter().any(|child| child.status == "failed") {
        VectorValidationStatus::Failed
    } else if children.iter().any(|child| child.status == "skipped") {
        VectorValidationStatus::Skipped
    } else {
        VectorValidationStatus::Ok
    };
    let artifact = report_path("production-gate");
    let report = VectorSearchReport {
        schema_version: VECTOR_SEARCH_SCHEMA_VERSION,
        suite: "production-gate".to_owned(),
        status: status_name(status).to_owned(),
        reason: (status != VectorValidationStatus::Ok)
            .then(|| "one or more child workloads did not complete".to_owned()),
        vector_candidates: Default::default(),
        symbolic_baseline: empty_metrics("production-gate"),
        vector_search: None,
        vector_stage_metrics: Default::default(),
        scorer_variants: Vec::new(),
        pairs: Vec::new(),
        validation_bounds: request.bounds,
        validation_cost: VectorValidationCostSummary::default(),
        children,
    };
    let root = request.artifact_root();
    let artifact = write(&root, artifact, &report)?;
    Ok(VectorValidationOutcome {
        schema_version: VECTOR_SEARCH_SCHEMA_VERSION,
        status,
        suite: "production-gate".to_owned(),
        artifact: Some(artifact),
        reason: report.reason,
    })
}

fn write_partial(
    request: VectorValidationRequest,
    suite: &SuiteDefinition,
    baseline: &EvaluationMetrics,
    reason: String,
    _started: Instant,
) -> Result<VectorValidationOutcome> {
    let artifact = report_path(&suite.labels.suite);
    let report = VectorSearchReport {
        schema_version: VECTOR_SEARCH_SCHEMA_VERSION,
        suite: suite.labels.suite.clone(),
        status: "skipped".to_owned(),
        reason: Some(reason.clone()),
        vector_candidates: candidates::VectorCandidateSummary::skipped_for_budget(
            &request,
            suite.workspace.len(),
            comparison_count(suite),
            reason.clone(),
        ),
        symbolic_baseline: baseline.clone(),
        vector_search: None,
        vector_stage_metrics: Default::default(),
        scorer_variants: Vec::new(),
        pairs: Vec::new(),
        validation_bounds: request.bounds,
        validation_cost: VectorValidationCostSummary::default(),
        children: Vec::new(),
    };
    let root = request.artifact_root();
    let artifact = write(&root, artifact, &report)?;
    Ok(VectorValidationOutcome {
        schema_version: VECTOR_SEARCH_SCHEMA_VERSION,
        status: VectorValidationStatus::Skipped,
        suite: suite.labels.suite.clone(),
        artifact: Some(artifact),
        reason: Some(reason),
    })
}

fn load_suite(request: &VectorValidationRequest, reporter: &mut Reporter) -> Result<SuiteDefinition> {
    match request.suite.as_str() {
        "vector-fixture" => Ok(vector_fixture_suite()),
        "default" => indexed_suite(request, EvalSuite::Default, reporter),
        "hard-negatives" => indexed_suite(request, EvalSuite::HardNegatives, reporter),
        "manual-internal" => indexed_suite(request, EvalSuite::ManualInternal, reporter),
        "manual-mathlib" => indexed_suite(request, EvalSuite::ManualMathlib, reporter),
        other => Err(Error::InvalidRequest {
            message: format!("unsupported vector validation suite: {other}"),
        }),
    }
}

fn indexed_suite(
    request: &VectorValidationRequest,
    suite: EvalSuite,
    reporter: &mut Reporter,
) -> Result<SuiteDefinition> {
    let labels = load_builtin(suite)?;
    let definition = suite_definition(request, suite);
    let started = Instant::now();
    let cache_root = cache_root_for(&definition, reporter)?;
    let local_label = format!("vector-{}-workspace", suite.as_str());
    let local = build_or_load_index(
        &definition.workspace,
        &definition.module_root,
        &local_label,
        &definition.origin,
        definition.require_oleans,
        definition.build_before_index,
        IndexBuildKind::Local,
        &cache_root,
        reporter,
    )?;
    let workspace = local.hydrate(&local.all_handles()?)?;
    let comparison = if let Some(external) = &definition.external {
        let external_index = if suite == EvalSuite::ManualMathlib {
            build_or_load_project_mathlib_index(&definition, external, &cache_root, reporter)?
        } else {
            build_or_load_index(
                &external.workspace,
                &external.module_root,
                &external.label,
                &external.origin,
                external.require_oleans,
                definition.build_before_index && !external.require_oleans,
                IndexBuildKind::External,
                &cache_root,
                reporter,
            )?
        };
        external_index.hydrate(&external_index.all_handles()?)?
    } else {
        Vec::new()
    };
    Ok(SuiteDefinition {
        labels,
        workspace,
        comparison,
        index_load_ms: started.elapsed().as_millis(),
    })
}

struct IndexedSuiteDefinition {
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

fn suite_definition(request: &VectorValidationRequest, suite: EvalSuite) -> IndexedSuiteDefinition {
    let repo = crate::repo_root();
    match suite {
        EvalSuite::Default => IndexedSuiteDefinition {
            workspace: request
                .workspace
                .clone()
                .unwrap_or_else(|| repo.join("tests/fixtures/tiny")),
            module_root: "Tiny".to_owned(),
            origin: "workspace".to_owned(),
            external: Some(ExternalSuiteIndex {
                workspace: repo.join("tests/fixtures/external"),
                module_root: "External".to_owned(),
                label: "vector-default-external".to_owned(),
                origin: "external:fixture".to_owned(),
                require_oleans: false,
            }),
            mathlib_source_override: None,
            build_before_index: true,
            require_oleans: false,
        },
        EvalSuite::HardNegatives => IndexedSuiteDefinition {
            workspace: request
                .workspace
                .clone()
                .unwrap_or_else(|| repo.join("tests/fixtures/tiny")),
            module_root: "Tiny".to_owned(),
            origin: "workspace".to_owned(),
            external: Some(ExternalSuiteIndex {
                workspace: repo.join("tests/fixtures/external"),
                module_root: "External".to_owned(),
                label: "vector-hard-negatives-external".to_owned(),
                origin: "external:fixture".to_owned(),
                require_oleans: false,
            }),
            mathlib_source_override: None,
            build_before_index: true,
            require_oleans: false,
        },
        EvalSuite::ManualInternal => IndexedSuiteDefinition {
            workspace: request.workspace.clone().unwrap_or_default(),
            module_root: request.manual_module.clone().unwrap_or_else(|| "Workspace".to_owned()),
            origin: "workspace".to_owned(),
            external: None,
            mathlib_source_override: None,
            build_before_index: false,
            require_oleans: true,
        },
        EvalSuite::ManualMathlib => IndexedSuiteDefinition {
            workspace: request.workspace.clone().unwrap_or_default(),
            module_root: request.manual_module.clone().unwrap_or_else(|| "Workspace".to_owned()),
            origin: "workspace".to_owned(),
            external: Some(ExternalSuiteIndex {
                workspace: request.workspace.clone().unwrap_or_default(),
                module_root: "Mathlib".to_owned(),
                label: "vector-manual-mathlib".to_owned(),
                origin: "mathlib".to_owned(),
                require_oleans: true,
            }),
            mathlib_source_override: request.mathlib_workspace.clone(),
            build_before_index: false,
            require_oleans: true,
        },
        EvalSuite::ProductionGate => unreachable!("production-gate expanded before suite definition"),
    }
}

fn cache_root_for(definition: &IndexedSuiteDefinition, reporter: &mut Reporter) -> Result<PathBuf> {
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
    definition: &IndexedSuiteDefinition,
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

#[allow(clippy::too_many_arguments)]
fn build_or_load_index(
    workspace_root: &Path,
    module_root: &str,
    label: &str,
    origin: &str,
    require_oleans: bool,
    build_before_index: bool,
    kind: IndexBuildKind,
    cache_root: &Path,
    reporter: &mut Reporter,
) -> Result<OpenedIndex> {
    if build_before_index {
        lake_build(workspace_root)?;
    }
    let workspace = resolve(
        WorkspaceRequest {
            requested_root: workspace_root.to_path_buf(),
            module_root: Some(module_root.to_owned()),
        },
        reporter,
    )?;
    let store = IndexStore::new(cache_root.to_path_buf());
    store.build_or_reuse(
        IndexBuildRequest {
            workspace,
            execution_root: None,
            label: label.to_owned(),
            module_root: module_root.to_owned(),
            origin: origin.to_owned(),
            include_private: true,
            include_generated: false,
            require_oleans,
            force: false,
            kind,
        },
        &WorkerClient::for_indexing(),
        reporter,
    )?;
    Ok(store.resolve(IndexReference::Label(label.to_owned()))?)
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
        Err(Error::InvalidRequest {
            message: format!(
                "lake build failed in {}: {}{}",
                workspace_root.display(),
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout)
            ),
        })
    }
}

fn vector_fixture_suite() -> SuiteDefinition {
    let labels =
        parse_json(include_str!("../../eval/eval-data/vector-fixture.json")).expect("valid vector fixture labels");
    let workspace = vector_fixture_declarations();
    SuiteDefinition {
        labels,
        workspace,
        comparison: Vec::new(),
        index_load_ms: 0,
    }
}

fn vector_fixture_declarations() -> Vec<lean_dup_index::HydratedDeclaration> {
    let mut declarations = vec![
        fixture_declaration(
            "vf-query",
            "VectorFixture.vector_only_query",
            "theorem",
            "theorem ∀ n, n + 0 = n",
            "vf-query",
            "vf-query",
        ),
        fixture_declaration(
            "vf-match",
            "VectorFixture.vector_only_match",
            "theorem",
            "theorem ∀ k, k = k + 0",
            "vf-match",
            "vf-match",
        ),
        fixture_declaration(
            "sym-query",
            "VectorFixture.symbolic_only_query",
            "theorem",
            "theorem ∀ n, n + 0 = n",
            "same-symbolic",
            "same-symbolic",
        ),
        fixture_declaration(
            "sym-doc",
            "VectorFixture.symbolic_only_document",
            "theorem",
            "theorem ∀ n, n + 0 = n",
            "same-symbolic",
            "same-symbolic",
        ),
        fixture_declaration(
            "trap-a",
            "LexicalTrap.height",
            "theorem",
            "theorem height t ≤ height (node t u)",
            "trap-a",
            "trap-a",
        ),
        fixture_declaration(
            "trap-b",
            "LexicalTrap.height_not_duplicate",
            "theorem",
            "theorem width t ≤ width (node t u)",
            "trap-b",
            "trap-b",
        ),
        fixture_declaration_with_flags(
            "skip-generated",
            "VectorFixture.generated_helper",
            "theorem True",
            &["generated"],
            &[],
        ),
        fixture_declaration_with_visibility("skip-private", "VectorFixture.private_helper", "private"),
        fixture_declaration_with_statement("skip-missing", "VectorFixture.missing_statement", ""),
        fixture_declaration_with_low_signal("skip-low", "VectorFixture.low_signal"),
        fixture_declaration_with_flags(
            "skip-non-action",
            "VectorFixture.non_actionable",
            "theorem True",
            &["non-actionable"],
            &[],
        ),
        fixture_declaration(
            "skip-synthetic",
            "Synthetic.vector_noise",
            "theorem",
            "theorem True",
            "synthetic",
            "synthetic",
        ),
    ];
    for index in 0..72 {
        declarations.push(fixture_declaration(
            &format!("noise-{index}"),
            &format!("VectorNoise.noise_{index}"),
            "theorem",
            &format!("theorem noise_{index} : True"),
            &format!("noise-statement-{index}"),
            &format!("noise-conclusion-{index}"),
        ));
    }
    declarations
}

fn fixture_declaration(
    id: &str,
    name: &str,
    kind: &str,
    statement: &str,
    statement_fp: &str,
    conclusion_fp: &str,
) -> lean_dup_index::HydratedDeclaration {
    let module = name.rsplit_once('.').map(|(module, _)| module).unwrap_or("").to_owned();
    lean_dup_index::HydratedDeclaration {
        handle: DeclarationHandle::from_fixture_id(id),
        declaration_id: id.to_owned(),
        origin: "workspace".to_owned(),
        module,
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
        feature_version: "features.roles.v1".to_owned(),
        fingerprints: Fingerprints {
            statement: statement_fp.to_owned(),
            safe_binder_permutation: statement_fp.to_owned(),
            connective_shape: format!("fixture-connective-{statement_fp}"),
            conclusion_shape: conclusion_fp.to_owned(),
        },
        role_features: vec![RoleFeature {
            role: "conclusion_const".to_owned(),
            key: conclusion_fp.to_owned(),
            display: None,
        }],
        binder_count: 1,
        low_signal_markers: Vec::new(),
    }
}

fn fixture_declaration_with_flags(
    id: &str,
    name: &str,
    statement: &str,
    flags: &[&str],
    low_signal: &[&str],
) -> lean_dup_index::HydratedDeclaration {
    let mut declaration = fixture_declaration(id, name, "theorem", statement, id, id);
    declaration.status_flags = flags.iter().map(|flag| (*flag).to_owned()).collect();
    declaration.low_signal_markers = low_signal.iter().map(|marker| (*marker).to_owned()).collect();
    declaration
}

fn fixture_declaration_with_visibility(id: &str, name: &str, visibility: &str) -> lean_dup_index::HydratedDeclaration {
    let mut declaration = fixture_declaration(id, name, "theorem", "theorem True", id, id);
    declaration.visibility = visibility.to_owned();
    declaration
}

fn fixture_declaration_with_statement(id: &str, name: &str, statement: &str) -> lean_dup_index::HydratedDeclaration {
    fixture_declaration(id, name, "theorem", statement, id, id)
}

fn fixture_declaration_with_low_signal(id: &str, name: &str) -> lean_dup_index::HydratedDeclaration {
    fixture_declaration_with_flags(id, name, "theorem True", &[], &["low-signal"])
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

fn observed_from_symbolic(suite: &str, output: &SearchObservation, timings: TimingMetrics) -> ObservedRun {
    ObservedRun {
        suite: suite.to_owned(),
        pairs: output
            .pairs
            .iter()
            .map(|pair| ObservedPair {
                pair: lean_dup_eval::GoldPair::new(pair.left.clone(), pair.right.clone()),
                generated: pair.generated,
                symbolic_generated: pair.symbolic_generated,
                merged_generated: pair.merged_generated,
                ranked: pair.ranked,
                generation_policy: pair.generation_policy.clone(),
                rank: pair.rank,
                shown: pair.shown,
                origin: pair.origin.clone(),
                feature_families: pair.feature_families.clone(),
                candidate_sources: pair
                    .candidate_sources
                    .iter()
                    .map(|source| ObservedCandidateSource {
                        source_id: source.source_id.clone(),
                        source_family: match source.source_family {
                            lean_dup_search::SearchCandidateSourceFamily::Symbolic => "symbolic".to_owned(),
                            lean_dup_search::SearchCandidateSourceFamily::LeanSemantic => "lean-semantic".to_owned(),
                        },
                        pair_id: source.pair_id.clone(),
                        left_declaration_id: source.left_declaration_id.clone(),
                        right_declaration_id: source.right_declaration_id.clone(),
                        origin: source.origin.clone(),
                        generation_rank: source.generation_rank,
                        top_k_status: match source.top_k_status {
                            lean_dup_search::SearchCandidateTopKStatus::Selected => "selected".to_owned(),
                            lean_dup_search::SearchCandidateTopKStatus::GeneratedNotSelected => {
                                "generated-not-selected".to_owned()
                            }
                        },
                        top_k_saturated: source.top_k_saturated,
                        feature_families: source.feature_families.clone(),
                    })
                    .collect(),
                survived_shown_filter: pair.survived_shown_filter,
            })
            .collect(),
        visible_groups: CountMetric {
            found: output.visible_groups_found,
            total: output.visible_groups_total,
        },
        probe_unavailable: CountMetric { found: 0, total: 0 },
        semantic_verification: Default::default(),
        timings,
        peak_memory_bytes: perf::peak_rss_bytes(),
    }
}

fn suite_k_values(request: &VectorValidationRequest) -> Vec<usize> {
    let mut values = request.k_values.clone();
    if !values.contains(&10) {
        values.push(10);
    }
    values.sort_unstable();
    values.dedup();
    values
}

fn budget_violation(
    request: &VectorValidationRequest,
    query_count: usize,
    corpus_count: usize,
    started: Instant,
) -> Option<String> {
    if query_count > request.bounds.max_queries {
        return Some(format!(
            "query-count-budget-exceeded: {query_count} > {}",
            request.bounds.max_queries
        ));
    }
    if corpus_count > request.bounds.max_declarations {
        return Some(format!(
            "declaration-count-budget-exceeded: {corpus_count} > {}",
            request.bounds.max_declarations
        ));
    }
    if started.elapsed().as_millis() > request.bounds.max_runtime_ms {
        return Some("runtime-budget-exceeded-before-vector-generation".to_owned());
    }
    if perf::peak_rss_bytes().is_some_and(|rss| rss > request.bounds.max_rss_bytes) {
        return Some("rss-budget-exceeded-before-vector-generation".to_owned());
    }
    None
}

fn comparison_count(suite: &SuiteDefinition) -> usize {
    if suite.comparison.is_empty() {
        suite.workspace.len()
    } else {
        suite.comparison.len()
    }
}

fn cost_summary(
    summary: &candidates::VectorCandidateSummary,
    peak_rss_bytes: Option<u64>,
    artifact_path: Option<PathBuf>,
) -> VectorValidationCostSummary {
    VectorValidationCostSummary {
        peak_rss_bytes,
        rss_status: if peak_rss_bytes.is_some() {
            "available"
        } else {
            "unavailable"
        }
        .to_owned(),
        model_cache_bytes: summary.model_cache_bytes,
        text_vector_cache_bytes: summary.text_vector_cache_bytes,
        vector_corpus_bytes: summary.vector_corpus_bytes,
        eligible_corpus_size: summary.eligible_corpus_size,
        query_count: summary.query_declaration_count,
        top_k: summary.top_k,
        top_k_saturated: summary.top_k_saturated,
        cold_build_ms: if summary.corpus_status == Some(lean_dup_vector_index::VectorCorpusStatus::Reused) {
            0
        } else {
            summary.corpus_build_ms
        },
        warm_open_query_ms: summary.corpus_open_ms.saturating_add(summary.query_ms),
        artifact_path,
    }
}

fn empty_metrics(suite: &str) -> EvaluationMetrics {
    EvaluationMetrics {
        suite: suite.to_owned(),
        recall: Vec::new(),
        shown_queue_precision: CountMetric::default(),
        hard_negative_hits: CountMetric::default(),
        visible_groups: CountMetric::default(),
        probe_unavailable: CountMetric::default(),
        stage_metrics: Default::default(),
        candidate_count: 0,
        timings: TimingMetrics::default(),
        peak_memory_bytes: None,
    }
}

fn status_label(status: VectorCandidateStatus) -> &'static str {
    match status {
        VectorCandidateStatus::Skipped => "skipped",
        VectorCandidateStatus::Failed => "failed",
        VectorCandidateStatus::Ok => "ok",
    }
}

fn status_name(status: VectorValidationStatus) -> &'static str {
    match status {
        VectorValidationStatus::Skipped => "skipped",
        VectorValidationStatus::Failed => "failed",
        VectorValidationStatus::Ok => "ok",
    }
}

trait WithSuite {
    fn with_suite(self, suite: &str) -> Self;
}

impl WithSuite for VectorValidationRequest {
    fn with_suite(mut self, suite: &str) -> Self {
        self.suite = suite.to_owned();
        self
    }
}
