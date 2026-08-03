//! Module import-closure digests sourced from `.ilean` headers and Lean sources.
//!
//! A semantic probe verdict for a pair `(A, B)` is fully determined by the two
//! declarations' content, the prover semantics, and the transitive import
//! closures of the two declarations' modules — that closure is exactly the set
//! of declarations the elaborator can bring into scope (see
//! `docs/architecture/probe-cache-scoping.md`). This module computes a digest
//! of that closure without a Lean runtime: each built module's `.ilean` carries
//! its direct-import list (JSON), the transitive closure is a fixpoint over
//! those, and each member folds its Lean source digest into the closure digest.
//!
//! Resolution searches the workspace build directory and every
//! `.lake/packages/*` build directory, so dependency modules (mathlib and
//! friends) resolve alongside workspace modules. Module names are unique across
//! a Lake dependency graph in practice; on a collision the first root in search
//! order wins (the workspace itself first), which can over-scope but never
//! under-scope when the colliding modules share content.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use sha2::{Digest, Sha256};

/// Resolves module import closures and content digests for one workspace run.
///
/// All caches are interior-mutable and single-threaded: semantic verification
/// is sequential, and probe planning shares one resolver across all pairs so a
/// module's closure is computed at most once per audit.
#[derive(Debug)]
pub struct ModuleClosureResolver {
    /// Build-library roots in search order: the workspace first, then packages.
    build_roots: Vec<PathBuf>,
    /// Source-tree roots in search order (workspace first, then packages); each
    /// paired with a conventional `src` subdirectory fallback at lookup time.
    source_roots: Vec<PathBuf>,
    direct: RefCell<BTreeMap<String, Option<Vec<String>>>>,
    members: RefCell<BTreeMap<String, Rc<BTreeSet<String>>>>,
    content: RefCell<BTreeMap<String, String>>,
}

impl ModuleClosureResolver {
    /// Build a resolver for one audited workspace root.
    pub fn for_workspace(root: &Path) -> Self {
        let mut build_roots = vec![build_lib_root(root)];
        let mut source_roots = vec![root.to_path_buf()];
        let packages = root.join(".lake").join("packages");
        if let Ok(entries) = std::fs::read_dir(&packages) {
            let mut package_roots: Vec<PathBuf> = entries
                .filter_map(std::result::Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect();
            package_roots.sort();
            for package in package_roots {
                build_roots.push(build_lib_root(&package));
                source_roots.push(package);
            }
        }
        Self {
            build_roots,
            source_roots,
            direct: RefCell::new(BTreeMap::new()),
            members: RefCell::new(BTreeMap::new()),
            content: RefCell::new(BTreeMap::new()),
        }
    }

    /// Digest of the transitive import closure of `module`: every reachable
    /// module paired with its own content digest. Editing any file inside the
    /// closure changes it; editing anything outside cannot.
    pub fn closure_digest(&self, module: &str) -> String {
        let members = self.closure_members(module);
        let mut ingredients = String::new();
        for member in members.iter() {
            ingredients.push_str(member);
            ingredients.push('\n');
            ingredients.push_str(&self.content_digest(member));
            ingredients.push('\n');
        }
        hex_digest(ingredients.as_bytes())
    }

    /// The transitive import closure of `module`, including `module` itself.
    fn closure_members(&self, module: &str) -> Rc<BTreeSet<String>> {
        if let Some(cached) = self.members.borrow().get(module) {
            return cached.clone();
        }
        // Post-order DFS with an explicit stack; import graphs are deep but
        // not pathological, and memoized member sets keep this near-linear.
        let mut result = BTreeSet::new();
        let mut stack = vec![(module.to_owned(), false)];
        while let Some((current, expanded)) = stack.pop() {
            if result.contains(&current) {
                continue;
            }
            if expanded {
                result.insert(current.clone());
                let mut merged = BTreeSet::new();
                for import in self.direct_imports(&current) {
                    if let Some(children) = self.members.borrow().get(&import) {
                        merged.extend(children.iter().cloned());
                    }
                }
                merged.insert(current.clone());
                let merged = Rc::new(merged);
                result.extend(merged.iter().cloned());
                self.members.borrow_mut().insert(current, merged);
                continue;
            }
            if self.members.borrow().contains_key(&current) {
                continue;
            }
            stack.push((current.clone(), true));
            for import in self.direct_imports(&current) {
                if !self.members.borrow().contains_key(&import) && !result.contains(&import) {
                    stack.push((import, false));
                }
            }
        }
        self.members
            .borrow()
            .get(module)
            .cloned()
            .unwrap_or_else(|| Rc::new(result))
    }

    /// Direct imports of one module from its `.ilean` header; `None` when no
    /// built artifact is visible (unbuilt module — it contributes nothing to
    /// any closure because nothing could have elaborated against it).
    fn direct_imports(&self, module: &str) -> Vec<String> {
        if let Some(cached) = self.direct.borrow().get(module) {
            return cached.clone().unwrap_or_default();
        }
        let imports = self
            .ilean_path(module)
            .and_then(|path| parse_ilean_imports(&path).ok());
        self.direct.borrow_mut().insert(module.to_owned(), imports.clone());
        imports.unwrap_or_default()
    }

    /// Content digest of one module: its Lean source when visible, else its
    /// `.ilean` bytes (weaker — rebuilds move it — but still content-derived).
    fn content_digest(&self, module: &str) -> String {
        if let Some(cached) = self.content.borrow().get(module) {
            return cached.clone();
        }
        let digest = self
            .source_path(module)
            .and_then(|path| std::fs::read(&path).ok())
            .or_else(|| self.ilean_path(module).and_then(|path| std::fs::read(&path).ok()))
            .map(|bytes| hex_digest(&bytes))
            .unwrap_or_else(|| hex_digest(module.as_bytes()));
        self.content.borrow_mut().insert(module.to_owned(), digest.clone());
        digest
    }

    fn ilean_path(&self, module: &str) -> Option<PathBuf> {
        let relative = module_file_name(module, "ilean");
        self.build_roots
            .iter()
            .map(|root| root.join(&relative))
            .find(|path| path.is_file())
    }

    fn source_path(&self, module: &str) -> Option<PathBuf> {
        let relative = module_file_name(module, "lean");
        for root in &self.source_roots {
            let direct = root.join(&relative);
            if direct.is_file() {
                return Some(direct);
            }
            let under_src = root.join("src").join(&relative);
            if under_src.is_file() {
                return Some(under_src);
            }
        }
        None
    }
}

fn build_lib_root(package_root: &Path) -> PathBuf {
    package_root.join(".lake").join("build").join("lib").join("lean")
}

fn module_file_name(module: &str, extension: &str) -> PathBuf {
    let mut path = PathBuf::new();
    for component in module.split('.') {
        path.push(component);
    }
    path.set_extension(extension);
    path
}

/// Parse the direct-import list out of an `.ilean` JSON document without
/// depending on its full schema (`directImports` entries are
/// `[name, importAll?, isExported?, isMeta?]` in every version we support).
fn parse_ilean_imports(path: &Path) -> std::result::Result<Vec<String>, Error> {
    let bytes = std::fs::read(path)?;
    let document: serde_json::Value = serde_json::from_slice(&bytes)?;
    let imports = document
        .get("directImports")
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.as_array()?.first()?.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    Ok(imports)
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

type Error = Box<dyn std::error::Error + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::ModuleClosureResolver;
    use std::path::Path;

    fn write_ilean(root: &Path, module: &str, imports: &[&str]) {
        let relative = module.replace('.', "/");
        let path = root.join(".lake/build/lib/lean").join(relative + ".ilean");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let entries: Vec<String> = imports.iter().map(|name| format!("[\"{name}\",false,false,false]")).collect();
        let document = format!("{{\"decls\":{{}},\"directImports\":[{}],\"module\":\"{module}\",\"version\":5}}", entries.join(","));
        std::fs::write(path, document).unwrap();
    }

    #[test]
    fn closure_digest_tracks_edits_only_inside_the_closure() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        write_ilean(root, "App.Main", &["App.Lib"]);
        write_ilean(root, "App.Lib", &["Dep.Core"]);
        write_ilean(root, "App.Unrelated", &[]);
        let write = |module: &str, text: &str| {
            let path = root.join(module.replace('.', "/") + ".lean");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write("App.Main", "def main := 1");
        write("App.Lib", "def lib := 1");
        write("App.Unrelated", "def unrelated := 1");
        // Dep.Core lives in a package
        let package = root.join(".lake/packages/dep");
        write_ilean(&package, "Dep.Core", &[]);
        let package_source = package.join("Dep/Core.lean");
        std::fs::create_dir_all(package_source.parent().unwrap()).unwrap();
        std::fs::write(package_source, "def core := 1").unwrap();

        let resolver = ModuleClosureResolver::for_workspace(root);
        let before = resolver.closure_digest("App.Main");

        // Editing an unrelated module must not move the digest.
        write("App.Unrelated", "def unrelated := 2");
        let resolver = ModuleClosureResolver::for_workspace(root);
        assert_eq!(before, resolver.closure_digest("App.Main"));

        // Editing a module inside the closure must move it.
        write("App.Lib", "def lib := 2");
        let resolver = ModuleClosureResolver::for_workspace(root);
        assert_ne!(before, resolver.closure_digest("App.Main"));
    }
}
