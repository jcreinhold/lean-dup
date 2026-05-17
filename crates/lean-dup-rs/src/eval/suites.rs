use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use serde::Serialize;

use crate::cache;
use crate::cli::EvalSuite;
use crate::error::{Error, Result};
use crate::eval::labels::{GoldLabels, load_builtin};
use crate::eval::memory;
use crate::eval::scoring::{EvaluationMetrics, GoldPair, ObservedPair, ObservedRun, TimingMetrics, score_run};
use crate::index::{IndexBuildKind, IndexBuildRequest, IndexReference, IndexStore, OpenedIndex};
use crate::progress::Reporter;
use crate::retrieval::{CandidateExplanation, RetrievalOutput, retrieve_candidates};
use crate::worker::WorkerClient;
use crate::workspace::{WorkspaceRequest, resolve};

#[derive(Debug, Clone)]
pub(crate) struct EvalRequest {
    pub(crate) suite: EvalSuite,
    pub(crate) workspace: Option<PathBuf>,
    pub(crate) mathlib_workspace: Option<PathBuf>,
    pub(crate) k_values: Vec<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct EvaluationReport {
    pub(crate) status: &'static str,
    pub(crate) suite: String,
    pub(crate) metrics: EvaluationMetrics,
}

struct SuiteDefinition {
    suite: EvalSuite,
    workspace: PathBuf,
    module_root: String,
    origin: String,
    external: Option<ExternalSuiteIndex>,
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

pub(crate) fn run(request: EvalRequest, reporter: &mut Reporter) -> Result<EvaluationReport> {
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
    let external = match &definition.external {
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
    let output = match external {
        Some(external) => retrieve_candidates(&workspace_rows, &[external])?,
        None => retrieve_candidates(&workspace_rows, &[])?,
    };
    let retrieval_ms = retrieval_started.elapsed().as_millis();

    let observed = ObservedRun {
        suite: labels.suite.clone(),
        pairs: observed_pairs(&output),
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
        status: "ok",
        suite: labels.suite,
        metrics,
    })
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
                    .mathlib_workspace
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("/Users/jcreinhold/Code/mathlib4")),
                module_root: "Mathlib".to_owned(),
                label: "eval-kanproofs-mathlib".to_owned(),
                origin: "mathlib".to_owned(),
                require_oleans: true,
            }),
            build_before_index: false,
            require_oleans: true,
        },
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
    Ok(cache::resolve_cache(&workspace)?.root)
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
            label: request.label.to_owned(),
            module_root: request.module_root.to_owned(),
            origin: request.origin.to_owned(),
            include_private: true,
            include_generated: false,
            require_oleans: request.require_oleans,
            force: false,
            kind: request.kind,
        },
        &WorkerClient::new(),
        reporter,
    )?;
    store.resolve(IndexReference::Label(request.label.to_owned()))
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

fn observed_pairs(output: &RetrievalOutput) -> Vec<ObservedPair> {
    let mut pairs = Vec::new();
    for set in &output.candidate_sets {
        for (index, candidate) in set.candidates.iter().enumerate() {
            pairs.push(ObservedPair {
                pair: GoldPair::new(
                    set.anchor.qualified_name.clone(),
                    candidate.declaration.qualified_name.clone(),
                ),
                rank: index + 1,
                shown: is_shown_queue_candidate(&candidate.explanation),
            });
        }
    }
    pairs
}

fn is_shown_queue_candidate(explanation: &CandidateExplanation) -> bool {
    explanation.contributions.iter().any(|contribution| {
        matches!(
            contribution.kind.as_str(),
            "statement-fingerprint" | "safe-permutation-fingerprint" | "connective-fingerprint"
        )
    })
}

fn enforce_suite_gates(definition: &SuiteDefinition, labels: &GoldLabels, metrics: &EvaluationMetrics) -> Result<()> {
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
    if metrics.hard_negative_hits.found != 0 {
        return Err(Error::Eval {
            message: format!(
                "default suite hard-negative gate failed: {}/{} appeared in the shown queue",
                metrics.hard_negative_hits.found, metrics.hard_negative_hits.total
            ),
        });
    }
    Ok(())
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate lives under repo/crates/lean-dup-rs")
        .to_path_buf()
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{EvalRequest, run};
    use crate::cli::EvalSuite;
    use crate::progress::Reporter;

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
}
