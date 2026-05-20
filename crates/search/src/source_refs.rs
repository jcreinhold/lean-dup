use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use lean_dup_index::HydratedDeclaration;

const DEFAULT_MAX_FILE_BYTES: u64 = 1_000_000;
const DEFAULT_MAX_REFERENCES_PER_DECLARATION: usize = 256;

/// Facts learned from workspace source files for review guidance.
///
/// Callers provide declarations with source spans and receive import, caller,
/// and source-clone facts. This module owns bounded token scanning; callers do
/// not provide regexes or inspect source text themselves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceFacts {
    pub declarations: BTreeMap<String, DeclarationSourceFact>,
    pub files: BTreeMap<PathBuf, SourceFileFact>,
    pub diagnostics: Vec<String>,
}

impl SourceFacts {
    pub fn empty() -> Self {
        Self {
            declarations: BTreeMap::new(),
            files: BTreeMap::new(),
            diagnostics: Vec::new(),
        }
    }

    pub fn declaration(&self, declaration_id: &str) -> Option<&DeclarationSourceFact> {
        self.declarations.get(declaration_id)
    }

    pub fn caller_count(&self, declaration_id: &str) -> usize {
        self.declaration(declaration_id)
            .map(|fact| fact.references.len())
            .unwrap_or(0)
    }

    pub fn source_fingerprint(&self, declaration_id: &str) -> Option<&str> {
        self.declaration(declaration_id)
            .and_then(|fact| fact.source_fingerprint.as_deref())
    }

    pub fn import_status_for(&self, declaration_id: &str, target_module: &str) -> ImportStatus {
        let Some(declaration) = self.declaration(declaration_id) else {
            return ImportStatus::Unknown;
        };
        if declaration.module == target_module {
            return ImportStatus::Direct;
        }
        let Some(path) = &declaration.source_file else {
            return ImportStatus::Unknown;
        };
        let Some(file) = self.files.get(path) else {
            return ImportStatus::Unknown;
        };
        if file.unavailable {
            return ImportStatus::Unknown;
        }
        if file.imports.iter().any(|module| module == target_module) {
            ImportStatus::Direct
        } else {
            ImportStatus::Missing
        }
    }
}

/// Input for bounded source fact collection.
///
/// Callers choose which declarations need caller references. Imports and
/// source fingerprints remain available for every declaration; expensive
/// cross-file reference scans are reserved for review groups that will use
/// them.
#[derive(Debug, Clone)]
pub struct SourceFactInput<'a> {
    pub declarations: &'a [HydratedDeclaration],
    pub max_file_bytes: u64,
    pub max_references_per_declaration: usize,
    pub reference_scope: SourceReferenceScope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceReferenceScope {
    All,
    None,
    Only(BTreeSet<String>),
}

impl SourceReferenceScope {
    fn includes(&self, declaration_id: &str) -> bool {
        match self {
            Self::All => true,
            Self::None => false,
            Self::Only(ids) => ids.contains(declaration_id),
        }
    }
}

impl<'a> SourceFactInput<'a> {
    pub fn new(declarations: &'a [HydratedDeclaration]) -> Self {
        Self {
            declarations,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_references_per_declaration: DEFAULT_MAX_REFERENCES_PER_DECLARATION,
            reference_scope: SourceReferenceScope::All,
        }
    }

    pub fn without_references(mut self) -> Self {
        self.reference_scope = SourceReferenceScope::None;
        self
    }

    pub fn with_reference_declarations(mut self, declaration_ids: BTreeSet<String>) -> Self {
        self.reference_scope = SourceReferenceScope::Only(declaration_ids);
        self
    }
}

/// Source-level facts for one declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeclarationSourceFact {
    pub declaration_id: String,
    pub qualified_name: String,
    pub module: String,
    pub source_file: Option<PathBuf>,
    pub source_fingerprint: Option<String>,
    pub references: Vec<SourceReference>,
}

/// Source-level facts for one loaded file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceFileFact {
    pub path: PathBuf,
    pub imports: Vec<String>,
    pub unavailable: bool,
}

/// One bounded caller reference discovered by token-aware scanning.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct SourceReference {
    pub file: PathBuf,
    pub line: usize,
    pub column: usize,
    pub text: String,
}

/// Import availability for replacing a local declaration with a target module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImportStatus {
    Direct,
    Missing,
    Unknown,
}

/// Collect source facts for declarations that belong to workspace files.
pub fn collect_source_facts(input: SourceFactInput<'_>) -> SourceFacts {
    let mut facts = SourceFacts::empty();
    let files = source_files(input.declarations);
    let mut loaded = BTreeMap::new();

    for path in files {
        match SourceFile::read(&path, input.max_file_bytes) {
            Ok(file) => {
                facts.files.insert(
                    path.clone(),
                    SourceFileFact {
                        imports: file.imports.clone(),
                        unavailable: false,
                        path: path.clone(),
                    },
                );
                loaded.insert(path, file);
            }
            Err(message) => {
                facts.diagnostics.push(message);
                facts.files.insert(
                    path.clone(),
                    SourceFileFact {
                        path,
                        imports: Vec::new(),
                        unavailable: true,
                    },
                );
            }
        }
    }

    for declaration in input.declarations {
        let source_file = declaration.source_span.as_ref().map(|span| PathBuf::from(&span.file));
        let source_fingerprint = source_file.as_ref().and_then(|path| {
            let source = loaded.get(path)?;
            let span = declaration.source_span.as_ref()?;
            source.fingerprint_for(declaration, span.start.line as usize, span.end.line as usize)
        });
        let references = if input.reference_scope.includes(&declaration.declaration_id) {
            reference_tokens(declaration)
                .map(|tokens| references_to(declaration, &tokens, &loaded, input.max_references_per_declaration))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        facts.declarations.insert(
            declaration.declaration_id.clone(),
            DeclarationSourceFact {
                declaration_id: declaration.declaration_id.clone(),
                qualified_name: declaration.qualified_name.clone(),
                module: declaration.module.clone(),
                source_file,
                source_fingerprint,
                references,
            },
        );
    }

    facts
}

struct SourceFile {
    path: PathBuf,
    lines: Vec<String>,
    stripped_lines: Vec<String>,
    imports: Vec<String>,
}

impl SourceFile {
    fn read(path: &Path, max_file_bytes: u64) -> Result<Self, String> {
        let metadata =
            std::fs::metadata(path).map_err(|source| format!("{}: source unavailable ({source})", path.display()))?;
        if metadata.len() > max_file_bytes {
            return Err(format!(
                "{}: source skipped because it exceeds {} bytes",
                path.display(),
                max_file_bytes
            ));
        }
        let text = std::fs::read_to_string(path)
            .map_err(|source| format!("{}: source unavailable ({source})", path.display()))?;
        let lines = text.lines().map(str::to_owned).collect::<Vec<_>>();
        let stripped_lines = strip_comments_by_line(&lines);
        let imports = parse_imports(&stripped_lines);
        Ok(Self {
            path: path.to_path_buf(),
            lines,
            stripped_lines,
            imports,
        })
    }

    fn fingerprint_for(&self, declaration: &HydratedDeclaration, start_line: usize, end_line: usize) -> Option<String> {
        if start_line == 0 || end_line < start_line || end_line > self.stripped_lines.len() {
            return None;
        }
        let mut snippet = self.stripped_lines[start_line - 1..end_line].join("\n");
        if let Some(short) = short_name(&declaration.qualified_name)
            && !short.is_empty()
        {
            snippet = snippet.replacen(short, "_decl", 1);
        }
        let normalized = snippet.split_whitespace().collect::<Vec<_>>().join(" ");
        if normalized.is_empty() {
            None
        } else {
            Some(hex_digest(normalized.as_bytes()))
        }
    }

    fn references_to(
        &self,
        tokens: &[String],
        declaration_file: Option<&Path>,
        declaration_start: Option<usize>,
        declaration_end: Option<usize>,
        remaining: usize,
    ) -> Vec<SourceReference> {
        if remaining == 0 {
            return Vec::new();
        }
        let mut references = Vec::new();
        for (index, stripped) in self.stripped_lines.iter().enumerate() {
            let line = index + 1;
            if declaration_file == Some(self.path.as_path())
                && declaration_start.is_some_and(|start| line >= start)
                && declaration_end.is_some_and(|end| line <= end)
            {
                continue;
            }
            let Some(column) = first_token_match(stripped, tokens) else {
                continue;
            };
            references.push(SourceReference {
                file: self.path.clone(),
                line,
                column,
                text: self.lines.get(index).cloned().unwrap_or_default().trim().to_owned(),
            });
            if references.len() >= remaining {
                break;
            }
        }
        references
    }
}

fn source_files(declarations: &[HydratedDeclaration]) -> BTreeSet<PathBuf> {
    declarations
        .iter()
        .filter(|declaration| declaration.origin == "workspace")
        .filter_map(|declaration| declaration.source_span.as_ref().map(|span| PathBuf::from(&span.file)))
        .collect()
}

fn references_to(
    declaration: &HydratedDeclaration,
    tokens: &[String],
    files: &BTreeMap<PathBuf, SourceFile>,
    max_references: usize,
) -> Vec<SourceReference> {
    if declaration.origin != "workspace" {
        return Vec::new();
    }
    let declaration_file = declaration.source_span.as_ref().map(|span| PathBuf::from(&span.file));
    let declaration_start = declaration.source_span.as_ref().map(|span| span.start.line as usize);
    let declaration_end = declaration.source_span.as_ref().map(|span| span.end.line as usize);
    let mut references = Vec::new();
    for source in files.values() {
        let remaining = max_references.saturating_sub(references.len());
        if remaining == 0 {
            break;
        }
        references.extend(source.references_to(
            tokens,
            declaration_file.as_deref(),
            declaration_start,
            declaration_end,
            remaining,
        ));
    }
    references.sort();
    references.dedup();
    references
}

fn reference_tokens(declaration: &HydratedDeclaration) -> Option<Vec<String>> {
    if declaration.qualified_name.is_empty() {
        return None;
    }
    let mut tokens = BTreeSet::new();
    tokens.insert(declaration.qualified_name.clone());
    if !declaration.display_name.is_empty() {
        tokens.insert(declaration.display_name.clone());
    }
    if let Some(short) = short_name(&declaration.qualified_name)
        && !short.is_empty()
    {
        tokens.insert(short.to_owned());
    }
    let mut tokens = tokens.into_iter().collect::<Vec<_>>();
    tokens.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
    Some(tokens)
}

fn first_token_match(line: &str, tokens: &[String]) -> Option<usize> {
    for token in tokens {
        let mut search_start = 0;
        while let Some(offset) = line[search_start..].find(token) {
            let start = search_start + offset;
            let end = start + token.len();
            if token_boundary(line, start, end) {
                return Some(start + 1);
            }
            search_start = end;
        }
    }
    None
}

fn token_boundary(line: &str, start: usize, end: usize) -> bool {
    let before = line[..start].chars().next_back();
    let after = line[end..].chars().next();
    !before.is_some_and(is_lean_name_char) && !after.is_some_and(is_lean_name_char)
}

fn is_lean_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '\'' | '.')
}

fn strip_comments_by_line(lines: &[String]) -> Vec<String> {
    let mut stripped = Vec::with_capacity(lines.len());
    let mut depth = 0usize;
    for line in lines {
        let mut output = String::new();
        let bytes = line.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            let two = line.get(index..index + 2);
            if depth == 0 && two == Some("--") {
                break;
            }
            if two == Some("/-") {
                depth += 1;
                index += 2;
                continue;
            }
            if depth > 0 && two == Some("-/") {
                depth -= 1;
                index += 2;
                continue;
            }
            if depth == 0
                && let Some(ch) = line[index..].chars().next()
            {
                output.push(ch);
                index += ch.len_utf8();
            } else {
                index += 1;
            }
        }
        stripped.push(output);
    }
    stripped
}

fn parse_imports(lines: &[String]) -> Vec<String> {
    let mut imports = Vec::new();
    for line in lines {
        let stripped = line.trim();
        if let Some(rest) = stripped.strip_prefix("import ") {
            imports.extend(rest.split_whitespace().map(str::to_owned));
        }
    }
    imports.sort();
    imports.dedup();
    imports
}

fn short_name(qualified_name: &str) -> Option<&str> {
    qualified_name.rsplit('.').next()
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{ImportStatus, SourceFactInput, collect_source_facts};
    use lean_dup_index::{DeclarationHandle, HydratedDeclaration};
    use lean_dup_worker::{Fingerprints, SourcePoint, SourceSpan};

    #[test]
    fn token_scan_ignores_substrings_and_comments() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("Tiny.lean");
        std::fs::write(
            &path,
            r#"
import Target.Module
namespace Tiny
theorem target : True := by trivial
theorem target_suffix : True := by trivial
-- target
/- target -/
theorem caller : True := target
end Tiny
"#,
        )
        .unwrap();
        let declaration = declaration("workspace:Tiny:Tiny.target", "Tiny.target", &path, 4, 4);

        let facts = collect_source_facts(SourceFactInput::new(std::slice::from_ref(&declaration)));
        let fact = facts.declaration(&declaration.declaration_id).unwrap();

        assert_eq!(fact.references.len(), 1);
        assert_eq!(fact.references[0].line, 8);
        assert_eq!(
            facts.import_status_for(&declaration.declaration_id, "Target.Module"),
            ImportStatus::Direct
        );
    }

    #[test]
    fn reference_scope_skips_callers_without_losing_imports_or_fingerprints() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("Tiny.lean");
        std::fs::write(
            &path,
            r#"
import Target.Module
namespace Tiny
theorem target : True := by trivial
theorem caller : True := target
end Tiny
"#,
        )
        .unwrap();
        let declaration = declaration("workspace:Tiny:Tiny.target", "Tiny.target", &path, 4, 4);

        let facts = collect_source_facts(SourceFactInput::new(std::slice::from_ref(&declaration)).without_references());
        let fact = facts.declaration(&declaration.declaration_id).unwrap();

        assert!(fact.source_fingerprint.is_some());
        assert!(fact.references.is_empty());
        assert_eq!(
            facts.import_status_for(&declaration.declaration_id, "Target.Module"),
            ImportStatus::Direct
        );
    }

    fn declaration(id: &str, name: &str, path: &std::path::Path, start: u64, end: u64) -> HydratedDeclaration {
        HydratedDeclaration {
            handle: DeclarationHandle::from_fixture_id("h"),
            declaration_id: id.to_owned(),
            origin: "workspace".to_owned(),
            module: "Tiny".to_owned(),
            qualified_name: name.to_owned(),
            display_name: name.rsplit('.').next().unwrap().to_owned(),
            kind: "theorem".to_owned(),
            visibility: "public".to_owned(),
            modifiers: Vec::new(),
            source_span: Some(SourceSpan {
                file: path.display().to_string(),
                start: SourcePoint { line: start, column: 1 },
                end: SourcePoint { line: end, column: 20 },
            }),
            statement_text: "theorem target : True".to_owned(),
            docstring_text: None,
            definition_body_summary: None,
            status_flags: Vec::new(),
            feature_version: "features.roles.v1".to_owned(),
            fingerprints: Fingerprints {
                statement: "statement".to_owned(),
                safe_binder_permutation: "permutation".to_owned(),
                connective_shape: "connective".to_owned(),
                conclusion_shape: "conclusion".to_owned(),
            },
            role_features: Vec::new(),
            binder_count: 0,
            low_signal_markers: Vec::new(),
        }
    }
}
