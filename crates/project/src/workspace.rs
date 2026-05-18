use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use walkdir::WalkDir;

use lean_dup_diagnostics::progress::Reporter;
use lean_dup_diagnostics::read_to_string;

use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct WorkspaceRequest {
    pub requested_root: PathBuf,
    pub module_root: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceFile {
    pub module: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedWorkspace {
    pub requested_root: PathBuf,
    pub root: PathBuf,
    pub lakefile: PathBuf,
    pub module_roots: Vec<String>,
    pub selected_roots: Vec<String>,
    pub source_files: Vec<SourceFile>,
}

impl ResolvedWorkspace {
    pub fn lean_toolchain_path(&self) -> PathBuf {
        self.root.join("lean-toolchain")
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.root.join("lake-manifest.json")
    }
}

pub fn resolve(request: WorkspaceRequest, reporter: &mut Reporter) -> Result<ResolvedWorkspace> {
    reporter.event(
        "workspace",
        None,
        None,
        format!("resolving {}", request.requested_root.display()),
    );

    let requested_root = normalize_existing_or_candidate(&request.requested_root)?;
    let root = lake_root_for(&requested_root)?;
    let lakefile = lakefile_path(&root).expect("lake_root_for only returns Lake workspaces");
    let discovered_roots = discover_module_roots(&root, &lakefile)?;
    let selected_roots = match request.module_root {
        Some(module_root) => vec![module_root],
        None => discovered_roots.clone(),
    };
    let source_files = enumerate_sources(&root, &selected_roots)?;
    if source_files.is_empty() {
        return Err(Error::NoSourceFiles(root));
    }

    reporter.event(
        "workspace",
        Some(source_files.len() as u64),
        Some(source_files.len() as u64),
        format!(
            "resolved {} module root(s), {} source file(s)",
            selected_roots.len(),
            source_files.len()
        ),
    );

    Ok(ResolvedWorkspace {
        requested_root,
        root,
        lakefile,
        module_roots: discovered_roots,
        selected_roots,
        source_files,
    })
}

pub fn module_to_file(root: &Path, module: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    for part in module.split('.') {
        path.push(part);
    }
    path.set_extension("lean");
    path
}

pub fn olean_exists(root: &Path, module: &str) -> bool {
    let mut relative = PathBuf::new();
    for part in module.split('.') {
        relative.push(part);
    }
    relative.set_extension("olean");

    let build = root.join(".lake");
    let Ok(entries) = std::fs::read_dir(build) else {
        return false;
    };
    entries
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("build"))
        })
        .any(|path| path.join("lib").join("lean").join(&relative).exists())
}

fn normalize_existing_or_candidate(path: &Path) -> Result<PathBuf> {
    let expanded = expand_home(path);
    if expanded.exists() {
        expanded.canonicalize().map_err(|source| Error::Io {
            message: "could not canonicalize workspace path",
            path: expanded,
            source,
        })
    } else {
        Err(Error::WorkspaceMissing(expanded))
    }
}

fn expand_home(path: &Path) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path.to_path_buf();
    };
    if text == "~" {
        return home_dir().unwrap_or_else(|| path.to_path_buf());
    }
    if let Some(rest) = text.strip_prefix("~/")
        && let Some(home) = home_dir()
    {
        return home.join(rest);
    }
    path.to_path_buf()
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn lake_root_for(path: &Path) -> Result<PathBuf> {
    if lakefile_path(path).is_some() {
        return Ok(path.to_path_buf());
    }
    let nested = path.join("lean");
    if lakefile_path(&nested).is_some() {
        return Ok(nested);
    }
    Err(Error::NotLakeWorkspace(path.to_path_buf()))
}

fn lakefile_path(root: &Path) -> Option<PathBuf> {
    let toml = root.join("lakefile.toml");
    if toml.exists() {
        return Some(toml);
    }
    let lean = root.join("lakefile.lean");
    if lean.exists() {
        return Some(lean);
    }
    None
}

fn discover_module_roots(root: &Path, lakefile: &Path) -> Result<Vec<String>> {
    let mut roots = if lakefile.file_name().and_then(|name| name.to_str()) == Some("lakefile.toml") {
        discover_toml_roots(lakefile)?
    } else {
        discover_lean_lakefile_roots(lakefile)?
    };
    if roots.is_empty() {
        roots = discover_top_level_roots(root)?;
    }
    roots.sort();
    roots.dedup();
    if roots.is_empty() {
        return Err(Error::NoModuleRoots(root.to_path_buf()));
    }
    Ok(roots)
}

fn discover_toml_roots(lakefile: &Path) -> Result<Vec<String>> {
    let text = read_to_string(lakefile.to_path_buf())?;
    let parsed: toml::Value = toml::from_str(&text).map_err(|source| Error::LakefileToml {
        path: lakefile.to_path_buf(),
        source,
    })?;
    let roots = parsed
        .get("lean_lib")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("name"))
        .filter_map(toml::Value::as_str)
        .map(ToOwned::to_owned)
        .collect();
    Ok(roots)
}

fn discover_lean_lakefile_roots(lakefile: &Path) -> Result<Vec<String>> {
    let text = read_to_string(lakefile.to_path_buf())?;
    let roots = text
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            let rest = trimmed.strip_prefix("lean_lib ")?;
            let raw = rest.split_whitespace().next()?;
            let root = raw
                .trim_matches('`')
                .trim_start_matches('«')
                .trim_end_matches('»')
                .to_owned();
            (!root.is_empty()).then_some(root)
        })
        .collect();
    Ok(roots)
}

fn discover_top_level_roots(root: &Path) -> Result<Vec<String>> {
    let mut roots = Vec::new();
    for entry in std::fs::read_dir(root).map_err(|source| Error::Io {
        message: "could not read workspace directory",
        path: root.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| Error::Io {
            message: "could not read workspace directory entry",
            path: root.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.file_name().and_then(|name| name.to_str()) == Some("lakefile.lean") {
            continue;
        }
        if path.extension().and_then(|extension| extension.to_str()) == Some("lean")
            && let Some(stem) = path.file_stem().and_then(|stem| stem.to_str())
        {
            roots.push(stem.to_owned());
        }
    }
    Ok(roots)
}

fn enumerate_sources(root: &Path, module_roots: &[String]) -> Result<Vec<SourceFile>> {
    let mut sources = BTreeMap::<String, PathBuf>::new();
    for module_root in module_roots {
        let root_file = module_to_file(root, module_root);
        if root_file.exists() {
            sources.insert(module_root.clone(), root_file);
        }

        let mut module_dir = root.to_path_buf();
        for part in module_root.split('.') {
            module_dir.push(part);
        }
        if !module_dir.exists() {
            continue;
        }
        for entry in WalkDir::new(&module_dir)
            .into_iter()
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_type().is_file())
        {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("lean") {
                continue;
            }
            let relative = path.strip_prefix(root).map_err(|source| Error::Io {
                message: "could not relativize Lean source path",
                path: path.to_path_buf(),
                source: std::io::Error::other(source),
            })?;
            let mut parts: Vec<String> = relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect();
            if let Some(last) = parts.last_mut()
                && let Some(stripped) = last.strip_suffix(".lean")
            {
                *last = stripped.to_owned();
            }
            sources.insert(parts.join("."), path.to_path_buf());
        }
    }
    Ok(sources
        .into_iter()
        .map(|(module, path)| SourceFile { module, path })
        .collect())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{WorkspaceRequest, resolve};
    use lean_dup_diagnostics::progress::Reporter;

    #[test]
    fn discovers_toml_lakefile_roots_and_sources() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join("lakefile.toml"),
            r#"
name = "Fixture"
[[lean_lib]]
name = "Fixture"
[[lean_lib]]
name = "Other"
"#,
        )
        .unwrap();
        fs::write(temp.path().join("Fixture.lean"), "import Fixture.Basic\n").unwrap();
        fs::create_dir(temp.path().join("Fixture")).unwrap();
        fs::write(
            temp.path().join("Fixture").join("Basic.lean"),
            "theorem t : True := True.intro\n",
        )
        .unwrap();
        fs::write(temp.path().join("Other.lean"), "#check Nat\n").unwrap();

        let resolved = resolve(
            WorkspaceRequest {
                requested_root: temp.path().to_path_buf(),
                module_root: None,
            },
            &mut Reporter::new(false, false),
        )
        .unwrap();

        assert_eq!(resolved.module_roots, vec!["Fixture", "Other"]);
        let modules: Vec<_> = resolved
            .source_files
            .iter()
            .map(|source| source.module.as_str())
            .collect();
        assert_eq!(modules, vec!["Fixture", "Fixture.Basic", "Other"]);
    }

    #[test]
    fn discovers_simple_lean_lakefile_roots() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("lakefile.lean"), "lean_lib Demo where\n").unwrap();
        fs::write(temp.path().join("Demo.lean"), "#check Nat\n").unwrap();

        let resolved = resolve(
            WorkspaceRequest {
                requested_root: temp.path().to_path_buf(),
                module_root: None,
            },
            &mut Reporter::new(false, false),
        )
        .unwrap();

        assert_eq!(resolved.module_roots, vec!["Demo"]);
        assert_eq!(resolved.source_files[0].module, "Demo");
    }

    #[test]
    fn accepts_conventional_nested_lean_workspace() {
        let temp = TempDir::new().unwrap();
        let lean = temp.path().join("lean");
        fs::create_dir(&lean).unwrap();
        fs::write(lean.join("lakefile.lean"), "lean_lib Nested where\n").unwrap();
        fs::write(lean.join("Nested.lean"), "#check Nat\n").unwrap();

        let resolved = resolve(
            WorkspaceRequest {
                requested_root: temp.path().to_path_buf(),
                module_root: None,
            },
            &mut Reporter::new(false, false),
        )
        .unwrap();

        assert_eq!(resolved.root, lean.canonicalize().unwrap());
        assert_eq!(resolved.source_files[0].module, "Nested");
    }
}
