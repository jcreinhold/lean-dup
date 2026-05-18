use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::workspace::{self, ResolvedWorkspace, WorkspaceRequest};
use lean_dup_report::progress::Reporter;
use lean_dup_report::{Error, Result};

const MATHLIB_PACKAGE_PATH: &[&str] = &[".lake", "packages", "mathlib"];

/// Project-pinned mathlib source facts for indexing and comparison.
///
/// Callers receive only the execution root and the resolved mathlib source
/// workspace. Lake package layout, source enumeration, and progress wording are
/// owned here so audit and index commands do not grow mathlib-specific steps.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectMathlib {
    pub project: ResolvedWorkspace,
    pub source: ResolvedWorkspace,
}

impl ProjectMathlib {
    pub fn execution_root(&self) -> PathBuf {
        self.project.root.clone()
    }
}

pub fn resolve_project(
    project_root: PathBuf,
    source_override: Option<PathBuf>,
    reporter: &mut Reporter,
) -> Result<ProjectMathlib> {
    let project = workspace::resolve(
        WorkspaceRequest {
            requested_root: project_root,
            module_root: None,
        },
        reporter,
    )?;
    resolve_for_workspace(project, source_override, reporter)
}

pub fn resolve_for_workspace(
    project: ResolvedWorkspace,
    source_override: Option<PathBuf>,
    reporter: &mut Reporter,
) -> Result<ProjectMathlib> {
    let source_root = match source_override {
        Some(path) => canonicalize_existing(&path)?,
        None => {
            let mut path = project.root.clone();
            for segment in MATHLIB_PACKAGE_PATH {
                path.push(segment);
            }
            canonicalize_existing(&path)?
        }
    };

    reporter.event(
        "mathlib.resolve",
        None,
        None,
        format!(
            "using mathlib source {} with project environment {}",
            source_root.display(),
            project.root.display()
        ),
    );

    let source = workspace::resolve(
        WorkspaceRequest {
            requested_root: source_root,
            module_root: Some("Mathlib".to_owned()),
        },
        reporter,
    )?;
    reporter.event(
        "mathlib.resolve",
        Some(source.source_files.len() as u64),
        Some(source.source_files.len() as u64),
        format!("resolved pinned mathlib source files from {}", source.root.display()),
    );
    Ok(ProjectMathlib { project, source })
}

fn canonicalize_existing(path: &Path) -> Result<PathBuf> {
    if !path.exists() {
        return Err(Error::WorkspaceMissing(path.to_path_buf()));
    }
    path.canonicalize().map_err(|source| Error::Io {
        message: "could not canonicalize workspace path",
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::resolve_project;
    use lean_dup_report::progress::Reporter;

    #[test]
    fn project_resolver_uses_pinned_package_mathlib_sources() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join("lakefile.toml"),
            r#"[[lean_lib]]
name = "Project"
"#,
        )
        .unwrap();
        fs::write(temp.path().join("Project.lean"), "#check Nat\n").unwrap();
        let mathlib = temp.path().join(".lake/packages/mathlib");
        fs::create_dir_all(mathlib.join("Mathlib/Algebra")).unwrap();
        fs::write(
            mathlib.join("lakefile.toml"),
            r#"[[lean_lib]]
name = "Mathlib"
"#,
        )
        .unwrap();
        fs::write(mathlib.join("Mathlib.lean"), "import Mathlib.Algebra.Basic\n").unwrap();
        fs::write(mathlib.join("Mathlib/Algebra/Basic.lean"), "#check Nat\n").unwrap();

        let resolved = resolve_project(temp.path().to_path_buf(), None, &mut Reporter::new(false, false)).unwrap();

        assert_eq!(resolved.project.root, temp.path().canonicalize().unwrap());
        assert_eq!(resolved.source.root, mathlib.canonicalize().unwrap());
        let modules = resolved
            .source
            .source_files
            .iter()
            .map(|source| source.module.as_str())
            .collect::<Vec<_>>();
        assert_eq!(modules, vec!["Mathlib", "Mathlib.Algebra.Basic"]);
    }
}
