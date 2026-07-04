use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use serde::Serialize;

use lean_dup_diagnostics::perf;
use lean_dup_diagnostics::progress::Reporter;
use lean_dup_index::{HydratedDeclaration, IndexBuildKind, IndexBuildRequest, IndexReference, IndexStore, OpenedIndex};
use lean_dup_project::{WorkspaceRequest, resolve, resolve_project_mathlib};
use lean_dup_search::{
    SearchCandidateLossFact, SearchCandidateLossStage, SearchCandidateSourceFact, SearchCandidateSourceFamily,
    SearchCandidateTopKStatus, SearchObservation, SearchObservationRequest, SearchScoringVariant,
    SearchStageObservation, SearchTrackedPair, observe_search, observe_search_stages, rescore_observation,
};
use lean_dup_worker::WorkerClient;

use crate::EvalSuite;
use crate::eval::labels::{GoldLabelFact, GoldLabels, LabelFactSource, LabelPolarity, TypedGoldLabel, load_builtin};
use crate::eval::scorer_ablations::{self, ScorerAblationVariantReport};
use crate::eval::scoring::{
    CountMetric, EvaluationMetrics, GoldPair, ObservedCandidateLoss, ObservedPair, ObservedRun, RecallAtK,
    TimingMetrics, score_run,
};
use crate::eval::search_dataset;
use crate::eval::stage_metrics::SemanticVerificationStageMetrics;
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
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalOutput {
    pub status: String,
    pub suite: String,
    pub scorer_version: String,
    pub review_policy_version: String,
    pub metrics: EvaluationMetrics,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manual_prerequisites: Option<ManualSuitePrerequisites>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_resolution: Option<LabelResolutionReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_dataset_artifact: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scorer_ablation_artifact: Option<PathBuf>,
    #[serde(skip)]
    pub scorer_ablations: Vec<ScorerAblationVariantReport>,
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
    pub review_policy_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<EvaluationMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub manual: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manual_prerequisites: Option<ManualSuitePrerequisites>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_resolution: Option<LabelResolutionReport>,
    #[serde(skip)]
    pub scorer_ablations: Vec<ScorerAblationVariantReport>,
}

/// Stable label-resolution facts for a completed evaluation run.
///
/// Eval owns the mapping from label-file names to current declarations. The
/// report records resolution and stage survival without exposing index storage,
/// raw statements, or private cache paths.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LabelResolutionReport {
    pub status: LabelResolutionStatus,
    pub positives: LabelTraceCount,
    pub hard_negatives: LabelTraceCount,
    pub blockers: Vec<String>,
    pub traces: Vec<LabelTrace>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LabelResolutionStatus {
    Ok,
    Blocked,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct LabelTraceCount {
    pub resolved: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LabelTrace {
    pub left: String,
    pub right: String,
    pub polarity: LabelPolarity,
    pub match_class: crate::eval::labels::MatchClass,
    pub left_resolution: LabelEndpointResolution,
    pub right_resolution: LabelEndpointResolution,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_pair: Option<GoldPair>,
    pub generated: bool,
    pub ranked: bool,
    pub rank: Option<usize>,
    pub visible: bool,
    pub lost_layer: LabelLossLayer,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LabelEndpointResolution {
    pub requested: String,
    pub status: LabelEndpointStatus,
    pub candidates: Vec<LabelResolutionCandidate>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LabelEndpointStatus {
    Exact,
    DisplayUnique,
    Ambiguous,
    Missing,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LabelResolutionCandidate {
    pub qualified_name: String,
    pub origin: String,
    pub kind: String,
    pub visibility: String,
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LabelLossLayer {
    None,
    LabelResolution,
    Eligibility,
    CandidateGeneration,
    Ranking,
    Visibility,
}

/// Operator-visible prerequisite facts for a slow manual suite.
///
/// Manual suites may depend on private workspaces, compiled `.olean` files, and
/// project-pinned mathlib sources. Eval reports those prerequisites as stable
/// checks so a skipped suite is actionable without exposing cache layout,
/// worker transport rows, or scorer internals.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ManualSuitePrerequisites {
    pub suite: String,
    pub workspace_path: Option<PathBuf>,
    pub module_selector: String,
    pub workspace: PrerequisiteCheck,
    pub labels: PrerequisiteCheck,
    pub compiled_oleans: PrerequisiteCheck,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mathlib: Option<ManualMathlibPrerequisites>,
    pub next_command: String,
    pub blockers: Vec<String>,
}

/// One prerequisite check with a stable status and human-readable detail.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PrerequisiteCheck {
    pub status: PrerequisiteStatus,
    pub detail: String,
}

/// Stable status vocabulary for manual-suite prerequisite checks.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PrerequisiteStatus {
    Ok,
    Missing,
    Blocked,
    Unchecked,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ManualMathlibPrerequisites {
    pub source_workspace: PrerequisiteCheck,
    pub compiled_oleans: PrerequisiteCheck,
    pub external_comparison_artifacts: PrerequisiteCheck,
}

impl ManualSuitePrerequisites {
    fn is_satisfied(&self) -> bool {
        self.blockers.is_empty()
    }

    fn skip_reason(&self) -> String {
        if self.blockers.is_empty() {
            format!("manual suite prerequisites satisfied; run `{}`", self.next_command)
        } else {
            format!("{}; next command: {}", self.blockers.join("; "), self.next_command)
        }
    }

    fn with_runtime_blocker(mut self, reason: String) -> Self {
        self.blockers.push(reason);
        self
    }
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
    if matches!(request.suite, EvalSuite::ManualInternal | EvalSuite::ManualMathlib) {
        return run_manual_single(request, reporter);
    }
    run_single(request, reporter)
}

fn run_manual_single(request: EvalRequest, reporter: &mut Reporter) -> Result<EvalOutput> {
    let prerequisites = manual_prerequisites(&request, reporter);
    if !prerequisites.is_satisfied() {
        let suite = request.suite.as_str().to_owned();
        let scorer_version = lean_dup_search::SearchScoringSummary::new(SearchScoringVariant::AllFeatures)
            .version
            .to_owned();
        return Ok(EvalOutput {
            status: "skipped".to_owned(),
            suite: suite.clone(),
            scorer_version: scorer_version.clone(),
            review_policy_version: "lean-dup.symbolic-review-policy.v2".to_owned(),
            metrics: aggregate_metrics(&suite, &[]),
            manual_prerequisites: Some(prerequisites.clone()),
            label_resolution: None,
            search_dataset_artifact: None,
            scorer_ablation_artifact: None,
            scorer_ablations: Vec::new(),
            runs: vec![EvaluationRunReport {
                suite,
                status: "skipped".to_owned(),
                scorer_version: Some(scorer_version),
                review_policy_version: Some("lean-dup.symbolic-review-policy.v2".to_owned()),
                metrics: None,
                reason: Some(prerequisites.skip_reason()),
                manual: true,
                manual_prerequisites: Some(prerequisites),
                label_resolution: None,
                scorer_ablations: Vec::new(),
            }],
        });
    }

    let mut output = run_single(request, reporter)?;
    output.manual_prerequisites = Some(prerequisites);
    Ok(output)
}

fn run_single(request: EvalRequest, reporter: &mut Reporter) -> Result<EvalOutput> {
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

    let label_resolution_input = resolve_label_references(&labels, request.suite, &workspace_rows, external.as_ref())?;
    let labels = label_resolution_input.labels;
    let tracked_pairs = tracked_pairs(&labels);
    let retrieval_started = Instant::now();
    let needs_detailed_observation = write_search_dataset || write_scorer_ablations;
    let (base_output, compact_output) = match &external {
        Some(external) if needs_detailed_observation => (
            Some(observe_search(SearchObservationRequest {
                workspace: &workspace_rows,
                comparison_indexes: std::slice::from_ref(external),
                tracked_pairs: &tracked_pairs,
                scoring_variant: SearchScoringVariant::AllFeatures,
            })?),
            None,
        ),
        Some(external) => (
            None,
            Some(observe_search_stages(SearchObservationRequest {
                workspace: &workspace_rows,
                comparison_indexes: std::slice::from_ref(external),
                tracked_pairs: &tracked_pairs,
                scoring_variant: SearchScoringVariant::AllFeatures,
            })?),
        ),
        None if needs_detailed_observation => (
            Some(observe_search(SearchObservationRequest {
                workspace: &workspace_rows,
                comparison_indexes: &[],
                tracked_pairs: &tracked_pairs,
                scoring_variant: SearchScoringVariant::AllFeatures,
            })?),
            None,
        ),
        None => (
            None,
            Some(observe_search_stages(SearchObservationRequest {
                workspace: &workspace_rows,
                comparison_indexes: &[],
                tracked_pairs: &tracked_pairs,
                scoring_variant: SearchScoringVariant::AllFeatures,
            })?),
        ),
    };
    let retrieval_ms = retrieval_started.elapsed().as_millis();
    drop(workspace_rows);
    drop(handles);
    drop(local);
    drop(external);
    let scorer_version = base_output
        .as_ref()
        .map(|output| output.scoring.version)
        .or_else(|| compact_output.as_ref().map(|output| output.scoring.version))
        .expect("search observation was produced")
        .to_owned();
    let review_policy_version = base_output
        .as_ref()
        .map(|output| output.review_policy.version)
        .or_else(|| compact_output.as_ref().map(|output| output.review_policy.version))
        .expect("search observation was produced")
        .to_owned();

    let observed_pairs = match (&base_output, &compact_output) {
        (Some(output), None) => observed_pairs(output),
        (None, Some(output)) => compact_observed_pairs(output),
        _ => unreachable!("exactly one search observation mode is selected"),
    };
    let candidate_losses = match (&base_output, &compact_output) {
        (Some(output), None) => observed_candidate_losses(&output.candidate_losses),
        (None, Some(output)) => observed_candidate_losses(&output.candidate_losses),
        _ => unreachable!("exactly one search observation mode is selected"),
    };
    let label_resolution = trace_labels(&labels, label_resolution_input.traces, &observed_pairs, request.suite);
    let observed = ObservedRun {
        suite: labels.suite.clone(),
        pairs: observed_pairs,
        candidate_losses,
        visible_groups: CountMetric {
            found: base_output
                .as_ref()
                .map(|output| output.visible_groups_found)
                .or_else(|| compact_output.as_ref().map(|output| output.visible_groups_found))
                .expect("search observation was produced"),
            total: base_output
                .as_ref()
                .map(|output| output.visible_groups_total)
                .or_else(|| compact_output.as_ref().map(|output| output.visible_groups_total))
                .expect("search observation was produced"),
        },
        probe_unavailable: CountMetric { found: 0, total: 0 },
        semantic_verification: SemanticVerificationStageMetrics {
            semantic_reranking: base_output
                .as_ref()
                .map(|output| output.semantic_reranking.clone())
                .or_else(|| compact_output.as_ref().map(|output| output.semantic_reranking.clone()))
                .expect("search observation was produced"),
            obligation_yield: base_output
                .as_ref()
                .map(|output| output.semantic_obligation_yield.clone())
                .or_else(|| {
                    compact_output
                        .as_ref()
                        .map(|output| output.semantic_obligation_yield.clone())
                })
                .expect("search observation was produced"),
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
    enforce_suite_gates(&definition, &labels, &metrics)?;
    let search_dataset_artifact = if write_search_dataset {
        let output = base_output
            .as_ref()
            .expect("search dataset requests use detailed search observations");
        let dataset = search_dataset::build(&labels.suite, &labels, output);
        Some(search_dataset::write_default_artifact(&repo_root(), &dataset)?)
    } else {
        None
    };
    let scorer_ablations = if write_scorer_ablations {
        let output = base_output
            .as_ref()
            .expect("scorer ablations use detailed search observations");
        scorer_ablation_variants(&labels, output, &k_values, index_load_ms, reporter)
    } else {
        Vec::new()
    };
    let scorer_ablation_artifact = if write_scorer_ablations {
        let output = base_output
            .as_ref()
            .expect("scorer ablations use detailed search observations");
        let report = scorer_ablations::report(
            &labels.suite,
            &scorer_version,
            &review_policy_version,
            output.semantic_reranking.clone(),
            output.semantic_obligation_yield.clone(),
            scorer_ablations.clone(),
            Vec::new(),
        );
        Some(scorer_ablations::write_default_artifact(&repo_root(), &report)?)
    } else {
        None
    };
    let status = if matches!(request.suite, EvalSuite::ManualInternal | EvalSuite::ManualMathlib)
        && label_resolution.status == LabelResolutionStatus::Blocked
    {
        "blocked"
    } else {
        "ok"
    };
    let label_resolution =
        matches!(request.suite, EvalSuite::ManualInternal | EvalSuite::ManualMathlib).then_some(label_resolution);
    Ok(EvalOutput {
        status: status.to_owned(),
        suite: labels.suite,
        scorer_version,
        review_policy_version,
        metrics,
        manual_prerequisites: None,
        label_resolution,
        search_dataset_artifact,
        scorer_ablation_artifact,
        scorer_ablations,
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

struct LabelResolutionInput {
    labels: GoldLabels,
    traces: Vec<LabelTraceSeed>,
}

#[derive(Debug, Clone)]
struct LabelTraceSeed {
    label: TypedGoldLabel,
    original_pair: GoldPair,
    left_resolution: LabelEndpointResolution,
    right_resolution: LabelEndpointResolution,
    canonical_pair: Option<GoldPair>,
}

fn resolve_label_references(
    labels: &GoldLabels,
    suite: EvalSuite,
    workspace: &[HydratedDeclaration],
    external: Option<&OpenedIndex>,
) -> Result<LabelResolutionInput> {
    let requested = labels
        .typed_pairs
        .iter()
        .flat_map(|label| [label.pair.left.clone(), label.pair.right.clone()])
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let index = LabelDeclarationIndex::build(workspace, external, &requested)?;
    let mut canonical_by_pair = BTreeMap::<GoldPair, GoldPair>::new();
    let mut traces = Vec::with_capacity(labels.typed_pairs.len());
    let mut canonical_typed_pairs = Vec::with_capacity(labels.typed_pairs.len());
    for typed in &labels.typed_pairs {
        let left_resolution = index.resolve(&typed.pair.left);
        let right_resolution = index.resolve(&typed.pair.right);
        let canonical_pair = canonical_pair(suite, &left_resolution, &right_resolution);
        let mut canonical = typed.clone();
        if let Some(pair) = &canonical_pair {
            canonical.pair = pair.clone();
            canonical_by_pair.insert(typed.pair.clone(), pair.clone());
        }
        canonical_typed_pairs.push(canonical.clone());
        traces.push(LabelTraceSeed {
            label: canonical,
            original_pair: typed.pair.clone(),
            left_resolution,
            right_resolution,
            canonical_pair,
        });
    }
    let mut canonical_label_facts = Vec::with_capacity(labels.label_facts.len());
    for fact in &labels.label_facts {
        let mut fact = fact.clone();
        if let Some(pair) = canonical_by_pair.get(&fact.pair) {
            fact.pair = pair.clone();
            if let Some(typed) = fact.typed.as_mut() {
                typed.pair = pair.clone();
            }
        }
        canonical_label_facts.push(fact);
    }
    Ok(LabelResolutionInput {
        labels: rebuild_labels(&labels.suite, canonical_typed_pairs, canonical_label_facts),
        traces,
    })
}

struct LabelDeclarationIndex {
    exact: BTreeMap<String, Vec<LabelResolutionCandidate>>,
    display: BTreeMap<String, Vec<LabelResolutionCandidate>>,
}

impl LabelDeclarationIndex {
    fn build(workspace: &[HydratedDeclaration], external: Option<&OpenedIndex>, requested: &[String]) -> Result<Self> {
        let mut exact = BTreeMap::<String, Vec<LabelResolutionCandidate>>::new();
        let mut display = BTreeMap::<String, Vec<LabelResolutionCandidate>>::new();
        for declaration in workspace {
            insert_candidate(&mut exact, &declaration.qualified_name, declaration);
            insert_candidate(&mut display, &declaration.display_name, declaration);
        }
        if let Some(external) = external {
            for declaration in external.declarations_named(requested)? {
                insert_candidate(&mut exact, &declaration.qualified_name, &declaration);
                insert_candidate(&mut display, &declaration.display_name, &declaration);
            }
            for declaration in external.declarations_with_display_names(requested)? {
                insert_candidate(&mut exact, &declaration.qualified_name, &declaration);
                insert_candidate(&mut display, &declaration.display_name, &declaration);
            }
        }
        for candidates in exact.values_mut().chain(display.values_mut()) {
            candidates.sort_by(|left, right| {
                left.origin
                    .cmp(&right.origin)
                    .then_with(|| left.qualified_name.cmp(&right.qualified_name))
            });
            candidates
                .dedup_by(|left, right| left.origin == right.origin && left.qualified_name == right.qualified_name);
        }
        Ok(Self { exact, display })
    }

    fn resolve(&self, name: &str) -> LabelEndpointResolution {
        if let Some(candidates) = self.exact.get(name) {
            return endpoint_resolution(name, candidates, LabelEndpointStatus::Exact);
        }
        if let Some(candidates) = self.display.get(name) {
            return if candidates.len() == 1 {
                endpoint_resolution(name, candidates, LabelEndpointStatus::DisplayUnique)
            } else {
                endpoint_resolution(name, candidates, LabelEndpointStatus::Ambiguous)
            };
        }
        LabelEndpointResolution {
            requested: name.to_owned(),
            status: LabelEndpointStatus::Missing,
            candidates: Vec::new(),
        }
    }
}

fn insert_candidate(
    map: &mut BTreeMap<String, Vec<LabelResolutionCandidate>>,
    key: &str,
    declaration: &HydratedDeclaration,
) {
    map.entry(key.to_owned()).or_default().push(LabelResolutionCandidate {
        qualified_name: declaration.qualified_name.clone(),
        origin: declaration.origin.clone(),
        kind: declaration.kind.clone(),
        visibility: declaration.visibility.clone(),
        skipped: skip_reasons(declaration),
    });
}

fn skip_reasons(declaration: &HydratedDeclaration) -> Vec<String> {
    let mut reasons = Vec::new();
    if declaration.status_flags.iter().any(|flag| flag == "generated") {
        reasons.push("generated".to_owned());
    }
    if declaration.visibility != "public" {
        reasons.push("non-public".to_owned());
    }
    if !declaration.low_signal_markers.is_empty() {
        reasons.push("low-signal".to_owned());
    }
    if declaration.statement_text.trim().is_empty() {
        reasons.push("missing-statement".to_owned());
    }
    if !matches!(declaration.kind.as_str(), "theorem" | "axiom" | "def" | "instance") {
        reasons.push("unsupported-kind".to_owned());
    }
    reasons
}

fn endpoint_resolution(
    requested: &str,
    candidates: &[LabelResolutionCandidate],
    status: LabelEndpointStatus,
) -> LabelEndpointResolution {
    LabelEndpointResolution {
        requested: requested.to_owned(),
        status: if candidates.len() == 1 {
            status
        } else {
            LabelEndpointStatus::Ambiguous
        },
        candidates: candidates.iter().take(8).cloned().collect(),
    }
}

fn canonical_pair(
    suite: EvalSuite,
    left: &LabelEndpointResolution,
    right: &LabelEndpointResolution,
) -> Option<GoldPair> {
    let left = unique_candidate(left)?;
    let right = unique_candidate(right)?;
    if suite == EvalSuite::ManualMathlib {
        let local_count = [left, right]
            .into_iter()
            .filter(|candidate| candidate.origin == "workspace")
            .count();
        let external_count = [left, right]
            .into_iter()
            .filter(|candidate| candidate.origin == "mathlib")
            .count();
        if local_count != 1 || external_count != 1 {
            return None;
        }
    }
    Some(GoldPair::new(left.qualified_name.clone(), right.qualified_name.clone()))
}

fn unique_candidate(resolution: &LabelEndpointResolution) -> Option<&LabelResolutionCandidate> {
    matches!(
        resolution.status,
        LabelEndpointStatus::Exact | LabelEndpointStatus::DisplayUnique
    )
    .then(|| resolution.candidates.first())
    .flatten()
}

fn rebuild_labels(suite: &str, typed_pairs: Vec<TypedGoldLabel>, mut label_facts: Vec<GoldLabelFact>) -> GoldLabels {
    if label_facts.is_empty() {
        label_facts = typed_pairs
            .iter()
            .cloned()
            .map(|typed| GoldLabelFact {
                pair: typed.pair.clone(),
                polarity: typed.polarity,
                source: LabelFactSource::TypedPair,
                typed: Some(typed),
            })
            .collect();
    }
    let positives = typed_pairs
        .iter()
        .filter(|label| label.polarity == LabelPolarity::Positive)
        .map(|label| label.pair.clone())
        .collect();
    let hard_negatives = typed_pairs
        .iter()
        .filter(|label| label.polarity == LabelPolarity::HardNegative)
        .map(|label| label.pair.clone())
        .collect();
    GoldLabels {
        suite: suite.to_owned(),
        positives,
        hard_negatives,
        typed_pairs,
        label_facts,
    }
}

fn trace_labels(
    labels: &GoldLabels,
    seeds: Vec<LabelTraceSeed>,
    observed_pairs: &[ObservedPair],
    suite: EvalSuite,
) -> LabelResolutionReport {
    let observed = observed_pairs
        .iter()
        .map(|pair| (pair.pair.clone(), pair))
        .collect::<BTreeMap<_, _>>();
    let mut blockers = BTreeSet::new();
    let traces = seeds
        .into_iter()
        .map(|seed| {
            let observed = seed
                .canonical_pair
                .as_ref()
                .and_then(|pair| observed.get(pair))
                .copied();
            let (generated, ranked, rank, visible) = observed.map_or((false, false, None, false), |pair| {
                (pair.generated, pair.ranked, pair.rank, pair.survived_shown_filter)
            });
            let (lost_layer, reason) = loss_reason(suite, &seed, generated, ranked, visible);
            if matches!(suite, EvalSuite::ManualInternal | EvalSuite::ManualMathlib)
                && seed.label.polarity == LabelPolarity::Positive
                && lost_layer != LabelLossLayer::None
            {
                blockers.insert(format!(
                    "{} / {} lost at {:?}: {}",
                    seed.original_pair.left, seed.original_pair.right, lost_layer, reason
                ));
            }
            LabelTrace {
                left: seed.original_pair.left,
                right: seed.original_pair.right,
                polarity: seed.label.polarity,
                match_class: seed.label.match_class,
                left_resolution: seed.left_resolution,
                right_resolution: seed.right_resolution,
                canonical_pair: seed.canonical_pair,
                generated,
                ranked,
                rank,
                visible,
                lost_layer,
                reason,
            }
        })
        .collect::<Vec<_>>();
    let positives_total = traces
        .iter()
        .filter(|trace| trace.polarity == LabelPolarity::Positive)
        .count();
    let hard_total = traces
        .iter()
        .filter(|trace| trace.polarity == LabelPolarity::HardNegative)
        .count();
    let positives_resolved = traces
        .iter()
        .filter(|trace| trace.polarity == LabelPolarity::Positive && trace.canonical_pair.is_some())
        .count();
    let hard_resolved = traces
        .iter()
        .filter(|trace| trace.polarity == LabelPolarity::HardNegative && trace.canonical_pair.is_some())
        .count();
    if matches!(suite, EvalSuite::ManualInternal | EvalSuite::ManualMathlib)
        && labels.positives.len() != positives_resolved
    {
        blockers.insert(format!(
            "manual suite has unresolved positive labels: {positives_resolved}/{positives_total} resolved"
        ));
    }
    let blockers = blockers.into_iter().collect::<Vec<_>>();
    LabelResolutionReport {
        status: if blockers.is_empty() {
            LabelResolutionStatus::Ok
        } else {
            LabelResolutionStatus::Blocked
        },
        positives: LabelTraceCount {
            resolved: positives_resolved,
            total: positives_total,
        },
        hard_negatives: LabelTraceCount {
            resolved: hard_resolved,
            total: hard_total,
        },
        blockers,
        traces,
    }
}

fn loss_reason(
    suite: EvalSuite,
    seed: &LabelTraceSeed,
    generated: bool,
    ranked: bool,
    visible: bool,
) -> (LabelLossLayer, String) {
    let left = unique_candidate(&seed.left_resolution);
    let right = unique_candidate(&seed.right_resolution);
    if left.is_none() || right.is_none() {
        return (
            LabelLossLayer::LabelResolution,
            "one or both label endpoints are missing or ambiguous".to_owned(),
        );
    }
    if suite == EvalSuite::ManualMathlib && seed.canonical_pair.is_none() {
        return (
            LabelLossLayer::LabelResolution,
            "manual-mathlib labels must resolve to one workspace declaration and one mathlib declaration".to_owned(),
        );
    }
    let skipped = left
        .into_iter()
        .chain(right)
        .flat_map(|candidate| candidate.skipped.iter().cloned())
        .collect::<BTreeSet<_>>();
    if !skipped.is_empty() {
        return (
            LabelLossLayer::Eligibility,
            format!(
                "resolved declaration is ineligible: {}",
                skipped.into_iter().collect::<Vec<_>>().join(", ")
            ),
        );
    }
    if !generated {
        return (
            LabelLossLayer::CandidateGeneration,
            "no symbolic retrieval feature generated this labeled pair".to_owned(),
        );
    }
    if !ranked {
        return (
            LabelLossLayer::Ranking,
            "pair was generated but did not enter the ranked queue".to_owned(),
        );
    }
    if !visible {
        return (
            LabelLossLayer::Visibility,
            "pair was ranked but hidden by review policy".to_owned(),
        );
    }
    (LabelLossLayer::None, "visible".to_owned())
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
            candidate_losses: observed_candidate_losses(&observation.candidate_losses),
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
            },
            true,
            reporter,
        ));
    }

    let completed_metrics = runs.iter().filter_map(|run| run.metrics.as_ref()).collect::<Vec<_>>();
    let metrics = aggregate_metrics("production-gate", &completed_metrics);
    let status = if runs.iter().any(|run| run.status == "failed") {
        "failed"
    } else if runs.iter().any(|run| run.status == "blocked") {
        "blocked"
    } else if runs.iter().any(|run| run.status == "skipped") {
        "incomplete"
    } else {
        "ok"
    };
    let scorer_version = runs
        .iter()
        .find_map(|run| run.scorer_version.clone())
        .unwrap_or_else(|| {
            lean_dup_search::SearchScoringSummary::new(SearchScoringVariant::AllFeatures)
                .version
                .to_owned()
        });
    let review_policy_version = runs
        .iter()
        .find_map(|run| run.review_policy_version.clone())
        .unwrap_or_else(|| "lean-dup.symbolic-review-policy.v2".to_owned());
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
            &review_policy_version,
            semantic_reranking,
            semantic_obligation_yield,
            scorer_ablations.clone(),
            children,
        );
        Some(scorer_ablations::write_default_artifact(&repo_root(), &report)?)
    } else {
        None
    };
    Ok(EvalOutput {
        status: status.to_owned(),
        suite: "production-gate".to_owned(),
        scorer_version,
        review_policy_version,
        metrics,
        manual_prerequisites: None,
        label_resolution: None,
        search_dataset_artifact: None,
        scorer_ablation_artifact,
        scorer_ablations,
        runs,
    })
}

fn run_child_suite(request: EvalRequest, manual: bool, reporter: &mut Reporter) -> EvaluationRunReport {
    let manual_prerequisites = manual.then(|| manual_prerequisites(&request, reporter));
    if let Some(prerequisites) = manual_prerequisites.as_ref()
        && !prerequisites.is_satisfied()
    {
        return EvaluationRunReport {
            suite: request.suite.as_str().to_owned(),
            status: "skipped".to_owned(),
            scorer_version: None,
            review_policy_version: None,
            metrics: None,
            reason: Some(prerequisites.skip_reason()),
            manual,
            manual_prerequisites,
            label_resolution: None,
            scorer_ablations: Vec::new(),
        };
    }

    let suite = request.suite;
    match run_single(request, reporter) {
        Ok(report) => EvaluationRunReport {
            suite: report.suite,
            status: report.status,
            scorer_version: Some(report.scorer_version),
            review_policy_version: Some(report.review_policy_version),
            metrics: Some(report.metrics),
            reason: None,
            manual,
            manual_prerequisites,
            label_resolution: report.label_resolution,
            scorer_ablations: report.scorer_ablations,
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
                scorer_version: None,
                review_policy_version: None,
                metrics: None,
                reason: Some(reason),
                manual,
                manual_prerequisites: manual_prerequisites.map(|prerequisites| {
                    if status == "skipped" {
                        prerequisites.with_runtime_blocker(error.to_string())
                    } else {
                        prerequisites
                    }
                }),
                label_resolution: None,
                scorer_ablations: Vec::new(),
            }
        }
    }
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

fn manual_module(request: &EvalRequest) -> String {
    request.manual_module.clone().unwrap_or_else(|| "Workspace".to_owned())
}

fn manual_prerequisites(request: &EvalRequest, reporter: &mut Reporter) -> ManualSuitePrerequisites {
    let module_selector = manual_module(request);
    let mut blockers = Vec::new();
    let labels = label_prerequisite(request.suite, &mut blockers);
    let next_command = manual_next_command(request, &module_selector);
    let workspace_path = request.workspace.clone();
    let (workspace, compiled_oleans, resolved_workspace) = match workspace_path.as_ref() {
        Some(path) => match resolve(
            WorkspaceRequest {
                requested_root: path.clone(),
                module_root: Some(module_selector.clone()),
            },
            reporter,
        ) {
            Ok(workspace) => {
                let missing = workspace
                    .missing_olean_sources(&workspace.root, &workspace.source_files)
                    .into_iter()
                    .map(|source| source.module.clone())
                    .collect::<Vec<_>>();
                let compiled = compiled_olean_check(&missing, "workspace");
                if !missing.is_empty() {
                    blockers.push(format!(
                        "missing compiled workspace oleans ({} missing; sample: {})",
                        missing.len(),
                        sample_modules(&missing)
                    ));
                }
                (
                    check_ok(format!(
                        "resolved {} Lean source files from {}",
                        workspace.source_files.len(),
                        workspace.root.display()
                    )),
                    compiled,
                    Some(workspace),
                )
            }
            Err(error) => {
                blockers.push(format!("workspace prerequisite failed: {error}"));
                (
                    PrerequisiteCheck {
                        status: PrerequisiteStatus::Missing,
                        detail: error.to_string(),
                    },
                    check_unchecked("compiled oleans require a resolved workspace"),
                    None,
                )
            }
        },
        None => {
            blockers.push("missing required --workspace <path> for manual suite".to_owned());
            (
                PrerequisiteCheck {
                    status: PrerequisiteStatus::Missing,
                    detail: "pass --workspace <path> for the manual corpus".to_owned(),
                },
                check_unchecked("compiled oleans require --workspace <path>"),
                None,
            )
        }
    };

    let mathlib = if request.suite == EvalSuite::ManualMathlib {
        Some(mathlib_prerequisites(
            request,
            resolved_workspace.as_ref(),
            reporter,
            &mut blockers,
        ))
    } else {
        None
    };

    ManualSuitePrerequisites {
        suite: request.suite.as_str().to_owned(),
        workspace_path,
        module_selector,
        workspace,
        labels,
        compiled_oleans,
        mathlib,
        next_command,
        blockers,
    }
}

fn label_prerequisite(suite: EvalSuite, blockers: &mut Vec<String>) -> PrerequisiteCheck {
    match load_builtin(suite) {
        Ok(labels) => check_ok(format!(
            "parsed built-in typed labels ({} positives, {} hard negatives)",
            labels.positives.len(),
            labels.hard_negatives.len()
        )),
        Err(error) => {
            blockers.push(format!("label prerequisite failed: {error}"));
            PrerequisiteCheck {
                status: PrerequisiteStatus::Blocked,
                detail: error.to_string(),
            }
        }
    }
}

fn mathlib_prerequisites(
    request: &EvalRequest,
    workspace: Option<&lean_dup_project::ResolvedWorkspace>,
    reporter: &mut Reporter,
    blockers: &mut Vec<String>,
) -> ManualMathlibPrerequisites {
    let Some(workspace) = workspace else {
        return ManualMathlibPrerequisites {
            source_workspace: check_unchecked("mathlib source requires a resolved project workspace"),
            compiled_oleans: check_unchecked("mathlib oleans require a resolved project workspace"),
            external_comparison_artifacts: check_unchecked("mathlib index reuse requires a resolved project workspace"),
        };
    };

    match resolve_project_mathlib(workspace.root.clone(), request.mathlib_workspace.clone(), reporter) {
        Ok(mathlib) => {
            let missing = mathlib
                .source
                .missing_olean_sources(&mathlib.source.root, &mathlib.source.source_files)
                .into_iter()
                .map(|source| source.module.clone())
                .collect::<Vec<_>>();
            let compiled = compiled_olean_check(&missing, "mathlib");
            if !missing.is_empty() {
                blockers.push(format!(
                    "missing compiled mathlib oleans ({} missing; sample: {})",
                    missing.len(),
                    sample_modules(&missing)
                ));
            }
            ManualMathlibPrerequisites {
                source_workspace: check_ok(format!(
                    "resolved {} mathlib source files from {}",
                    mathlib.source.source_files.len(),
                    mathlib.source.root.display()
                )),
                compiled_oleans: compiled,
                external_comparison_artifacts: check_ok(
                    "manual-mathlib builds or reuses the project-pinned mathlib index; no separate prebuilt comparison artifact is required",
                ),
            }
        }
        Err(error) => {
            blockers.push(format!("mathlib prerequisite failed: {error}"));
            ManualMathlibPrerequisites {
                source_workspace: PrerequisiteCheck {
                    status: PrerequisiteStatus::Missing,
                    detail: error.to_string(),
                },
                compiled_oleans: check_unchecked("mathlib oleans require a resolved mathlib source workspace"),
                external_comparison_artifacts: check_unchecked("mathlib index reuse requires resolved mathlib sources"),
            }
        }
    }
}

fn compiled_olean_check(missing: &[String], label: &str) -> PrerequisiteCheck {
    if missing.is_empty() {
        check_ok(format!("all selected {label} modules have compiled oleans"))
    } else {
        PrerequisiteCheck {
            status: PrerequisiteStatus::Missing,
            detail: format!("{} missing; sample: {}", missing.len(), sample_modules(missing)),
        }
    }
}

fn check_ok(detail: impl Into<String>) -> PrerequisiteCheck {
    PrerequisiteCheck {
        status: PrerequisiteStatus::Ok,
        detail: detail.into(),
    }
}

fn check_unchecked(detail: impl Into<String>) -> PrerequisiteCheck {
    PrerequisiteCheck {
        status: PrerequisiteStatus::Unchecked,
        detail: detail.into(),
    }
}

fn sample_modules(modules: &[String]) -> String {
    modules.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
}

fn manual_next_command(request: &EvalRequest, module_selector: &str) -> String {
    let workspace = request
        .workspace
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<manual-workspace>".to_owned());
    let mut command = format!(
        "cargo run -p lean-dup -- eval --suite {} --workspace {} --manual-module {} --format json --output target/eval/{}.json",
        request.suite.as_str(),
        workspace,
        module_selector,
        request.suite.as_str()
    );
    if let Some(mathlib_workspace) = &request.mathlib_workspace {
        command.push_str(&format!(" --mathlib-workspace {}", mathlib_workspace.display()));
    }
    command
}

fn is_manual_prerequisite_error(reason: &str) -> bool {
    reason.contains("missing compiled oleans")
        || (reason.contains("import_failed") && reason.contains("object file") && reason.contains("does not exist"))
        || reason.contains("workspace does not exist")
        || reason.contains("not a Lake workspace")
}

fn suite_k_values(suite: EvalSuite, requested: &[usize]) -> Vec<usize> {
    let mut values = requested.to_vec();
    if suite == EvalSuite::Default && !values.contains(&10) {
        values.push(10);
    }
    values
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
            max_heartbeats: None,
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
            max_heartbeats: None,
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
            merged_generated: pair.merged_generated,
            ranked: pair.ranked,
            generation_policy: pair.generation_policy.clone(),
            rank: pair.rank,
            shown: pair.shown,
            origin: pair.origin.clone(),
            feature_families: pair.feature_families.clone(),
            candidate_sources: observed_candidate_sources(&pair.candidate_sources),
            survived_shown_filter: pair.survived_shown_filter,
        })
        .collect()
}

fn compact_observed_pairs(output: &SearchStageObservation) -> Vec<ObservedPair> {
    output
        .pairs
        .iter()
        .map(|pair| ObservedPair {
            pair: GoldPair::new(pair.left.clone(), pair.right.clone()),
            generated: pair.generated,
            symbolic_generated: pair.symbolic_generated,
            merged_generated: pair.merged_generated,
            ranked: pair.ranked,
            generation_policy: pair.generation_policy.clone(),
            rank: pair.rank,
            shown: pair.shown,
            origin: pair.origin.clone(),
            feature_families: pair.feature_families.clone(),
            candidate_sources: observed_candidate_sources(&pair.candidate_sources),
            survived_shown_filter: pair.survived_shown_filter,
        })
        .collect()
}

fn observed_candidate_sources(
    items: &[SearchCandidateSourceFact],
) -> Vec<crate::eval::scoring::ObservedCandidateSource> {
    items
        .iter()
        .map(|source| crate::eval::scoring::ObservedCandidateSource {
            source_id: source.source_id.clone(),
            source_family: source_family_label(source.source_family).to_owned(),
            pair_id: source.pair_id.clone(),
            left_declaration_id: source.left_declaration_id.clone(),
            right_declaration_id: source.right_declaration_id.clone(),
            origin: source.origin.clone(),
            generation_rank: source.generation_rank,
            top_k_status: top_k_status_label(source.top_k_status).to_owned(),
            top_k_saturated: source.top_k_saturated,
            feature_families: source.feature_families.clone(),
        })
        .collect()
}

fn observed_candidate_losses(items: &[SearchCandidateLossFact]) -> Vec<ObservedCandidateLoss> {
    items
        .iter()
        .map(|loss| ObservedCandidateLoss {
            pair: GoldPair::new(loss.left.clone(), loss.right.clone()),
            loss_stage: loss_stage_label(loss.loss_stage).to_owned(),
            source_id: loss.source_id.clone(),
            source_family: source_family_label(loss.source_family).to_owned(),
            policy: loss.policy.clone(),
            source: loss.source.clone(),
            reason: loss.reason.clone(),
            feature_family: loss.feature_family.clone(),
            count: loss.count,
        })
        .collect()
}

fn source_family_label(family: SearchCandidateSourceFamily) -> &'static str {
    match family {
        SearchCandidateSourceFamily::Symbolic => "symbolic",
        SearchCandidateSourceFamily::LeanSemantic => "lean-semantic",
    }
}

fn loss_stage_label(stage: SearchCandidateLossStage) -> &'static str {
    match stage {
        SearchCandidateLossStage::FanoutPruned => "fanout-pruned",
    }
}

fn top_k_status_label(status: SearchCandidateTopKStatus) -> &'static str {
    match status {
        SearchCandidateTopKStatus::Selected => "selected",
        SearchCandidateTopKStatus::GeneratedNotSelected => "generated-not-selected",
    }
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
    use crate::eval::labels::{
        AdjudicationSource, ExpectedStageVisibility, GoldLabelFact, GoldLabels, LabelConfidence, LabelFactSource,
        LabelPolarity, MatchClass, TypedGoldLabel,
    };
    use crate::eval::scoring::{CountMetric, EvaluationMetrics, RecallAtK, TimingMetrics};
    use crate::eval::stage_metrics::{SearchStageMetrics, SemanticVerificationStageMetrics};
    use lean_dup_diagnostics::progress::Reporter;
    use lean_dup_index::{DeclarationHandle, HydratedDeclaration};
    use lean_dup_worker::Fingerprints;
    use rustc_hash::FxHashSet;

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
            },
            true,
            &mut Reporter::new(false, false),
        );

        assert_eq!(report.suite, "manual-internal");
        assert_eq!(report.status, "skipped");
        assert!(report.manual);
        let prerequisites = report.manual_prerequisites.expect("manual prerequisites");
        assert_eq!(prerequisites.workspace.status, super::PrerequisiteStatus::Missing);
        assert_eq!(prerequisites.labels.status, super::PrerequisiteStatus::Ok);
        assert_eq!(
            prerequisites.compiled_oleans.status,
            super::PrerequisiteStatus::Unchecked
        );
        assert!(prerequisites.next_command.contains("--workspace"));
        assert!(
            report
                .reason
                .as_deref()
                .unwrap()
                .contains("workspace prerequisite failed")
        );
    }

    #[test]
    fn manual_child_suite_without_workspace_reports_operator_command() {
        let report = run_child_suite(
            EvalRequest {
                suite: EvalSuite::ManualMathlib,
                workspace: None,
                mathlib_workspace: None,
                manual_module: Some("KanProofs".to_owned()),
                k_values: vec![1, 5, 10],
                write_search_dataset: false,
                write_scorer_ablations: false,
            },
            true,
            &mut Reporter::new(false, false),
        );

        assert_eq!(report.status, "skipped");
        let prerequisites = report.manual_prerequisites.expect("manual prerequisites");
        assert_eq!(prerequisites.module_selector, "KanProofs");
        assert_eq!(prerequisites.workspace.status, super::PrerequisiteStatus::Missing);
        assert_eq!(prerequisites.labels.status, super::PrerequisiteStatus::Ok);
        assert!(prerequisites.mathlib.is_some());
        assert!(
            prerequisites
                .next_command
                .contains("cargo run -p lean-dup -- eval --suite manual-mathlib")
        );
        assert!(
            prerequisites
                .blockers
                .iter()
                .any(|blocker| blocker.contains("missing required --workspace"))
        );
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
    fn manual_label_resolution_blocks_unresolved_positive_labels() {
        let labels = GoldLabels {
            suite: "manual-internal".to_owned(),
            positives: FxHashSet::default(),
            hard_negatives: FxHashSet::default(),
            typed_pairs: vec![
                typed_label("alpha", "beta", LabelPolarity::Positive),
                typed_label("dup", "missing", LabelPolarity::Positive),
            ],
            label_facts: vec![
                label_fact("alpha", "beta", LabelPolarity::Positive),
                label_fact("dup", "missing", LabelPolarity::Positive),
            ],
        };
        let workspace = vec![
            declaration("Pkg.A.alpha", "alpha", "workspace", &[]),
            declaration("Pkg.B.beta", "beta", "workspace", &[]),
            declaration("Pkg.C.dup", "dup", "workspace", &[]),
            declaration("Pkg.D.dup", "dup", "workspace", &[]),
        ];

        let resolved = super::resolve_label_references(&labels, EvalSuite::ManualInternal, &workspace, None).unwrap();
        let report = super::trace_labels(&resolved.labels, resolved.traces, &[], EvalSuite::ManualInternal);

        assert_eq!(report.status, super::LabelResolutionStatus::Blocked);
        assert_eq!(report.positives.resolved, 1);
        assert_eq!(report.positives.total, 2);
        assert!(
            report
                .blockers
                .iter()
                .any(|blocker| blocker.contains("manual suite has unresolved positive labels: 1/2 resolved"))
        );
        assert!(report.traces.iter().any(|trace| {
            trace.left == "alpha"
                && trace
                    .canonical_pair
                    .as_ref()
                    .is_some_and(|pair| pair.left == "Pkg.A.alpha" && pair.right == "Pkg.B.beta")
        }));
        assert!(report.traces.iter().any(|trace| trace.left == "dup"
            && trace.left_resolution.status == super::LabelEndpointStatus::Ambiguous
            && trace.lost_layer == super::LabelLossLayer::LabelResolution));
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
                candidate_source_recall: Default::default(),
                candidate_stage_recall: Default::default(),
                top_k_recall_before_final_ranking: vec![RecallAtK { k: 10, found, total }],
                ranked_recall: vec![RecallAtK { k: 10, found, total }],
                visible_queue_precision: CountMetric { found, total },
                hard_negative_survival: Default::default(),
                hard_negative_stage_survival: Default::default(),
                candidate_count_by_origin: Default::default(),
                candidate_count_by_feature_family: Default::default(),
                generated_candidate_count_by_source_family: Default::default(),
                generated_candidate_count_by_source_id: Default::default(),
                generated_candidate_count_by_policy: Default::default(),
                generated_candidate_count_by_feature_family: Default::default(),
                hard_negative_generated_by_feature_family: Default::default(),
                candidate_loss_metrics: Default::default(),
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

    fn typed_label(left: &str, right: &str, polarity: LabelPolarity) -> TypedGoldLabel {
        TypedGoldLabel {
            pair: super::GoldPair::new(left.to_owned(), right.to_owned()),
            polarity,
            match_class: if polarity == LabelPolarity::HardNegative {
                MatchClass::HardNegative
            } else {
                MatchClass::ExactTheoremDuplicate
            },
            expected_stage_visibility: ExpectedStageVisibility::Visible,
            adjudication_source: AdjudicationSource::ManualInspection,
            confidence: LabelConfidence::High,
            semantic_verification_required: true,
            static_evidence_acceptable: true,
        }
    }

    fn label_fact(left: &str, right: &str, polarity: LabelPolarity) -> GoldLabelFact {
        let typed = typed_label(left, right, polarity);
        GoldLabelFact {
            pair: typed.pair.clone(),
            polarity,
            source: LabelFactSource::TypedPair,
            typed: Some(typed),
        }
    }

    fn declaration(
        qualified_name: &str,
        display_name: &str,
        origin: &str,
        skipped_markers: &[&str],
    ) -> HydratedDeclaration {
        HydratedDeclaration {
            handle: DeclarationHandle::from_fixture_id(qualified_name),
            declaration_id: qualified_name.to_owned(),
            origin: origin.to_owned(),
            module: "Pkg".to_owned(),
            qualified_name: qualified_name.to_owned(),
            display_name: display_name.to_owned(),
            kind: "theorem".to_owned(),
            visibility: "public".to_owned(),
            modifiers: Vec::new(),
            source_span: None,
            statement_text: "∀ x, x = x".to_owned(),
            docstring_text: None,
            definition_body_summary: None,
            status_flags: Vec::new(),
            feature_version: "fixture".to_owned(),
            fingerprints: Fingerprints {
                statement: qualified_name.to_owned(),
                safe_binder_permutation: qualified_name.to_owned(),
                connective_shape: qualified_name.to_owned(),
                conclusion_shape: qualified_name.to_owned(),
            },
            role_features: Vec::new(),
            binder_count: 0,
            low_signal_markers: skipped_markers.iter().map(|marker| (*marker).to_owned()).collect(),
        }
    }
}
