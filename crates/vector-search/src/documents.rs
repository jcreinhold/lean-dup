use lean_dup_embedding::EmbeddingInputPolicy;
use lean_dup_index::HydratedDeclaration;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{Error, Result};

pub(crate) const DOCUMENT_POLICY_VERSION: &str = "lean-dup.embedding-document.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum DocumentPolicy {
    Statement,
    NameAndStatement,
    DefinitionAware,
    DocstringAugmented,
}

impl DocumentPolicy {
    pub(crate) fn from_id(id: &str) -> Result<Self> {
        match id {
            "statement" => Ok(Self::Statement),
            "name-and-statement" => Ok(Self::NameAndStatement),
            "definition-aware" => Ok(Self::DefinitionAware),
            "docstring-augmented" => Ok(Self::DocstringAugmented),
            other => Err(Error::InvalidRequest {
                message: format!("unsupported document policy: {other}"),
            }),
        }
    }

    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Statement => "statement",
            Self::NameAndStatement => "name-and-statement",
            Self::DefinitionAware => "definition-aware",
            Self::DocstringAugmented => "docstring-augmented",
        }
    }

    pub(crate) fn embedding_policy(self) -> EmbeddingInputPolicy {
        EmbeddingInputPolicy {
            policy_id: self.id().to_owned(),
            version: DOCUMENT_POLICY_VERSION.to_owned(),
            includes_declaration_name: matches!(
                self,
                Self::NameAndStatement | Self::DefinitionAware | Self::DocstringAugmented
            ),
            includes_statement: true,
            includes_definition_body_summary: matches!(self, Self::DefinitionAware | Self::DocstringAugmented),
            includes_docstring: matches!(self, Self::DocstringAugmented),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SemanticDocuments {
    pub(crate) policy: DocumentPolicy,
    pub(crate) documents: Vec<SemanticDocument>,
    pub(crate) availability: ContentAvailability,
}

#[derive(Debug, Clone)]
pub(crate) struct SemanticDocument {
    pub(crate) declaration_name: String,
    pub(crate) module_name: String,
    pub(crate) declaration_kind: String,
    pub(crate) content_hash: String,
    pub(crate) text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct ContentAvailability {
    pub(crate) total: usize,
    pub(crate) with_statement: usize,
    pub(crate) with_docstring: usize,
    pub(crate) with_definition_body_summary: usize,
}

pub(crate) fn build(policy: DocumentPolicy, declarations: &[HydratedDeclaration]) -> SemanticDocuments {
    let mut availability = ContentAvailability::default();
    let documents = declarations
        .iter()
        .map(|declaration| document(policy, declaration, &mut availability))
        .collect();
    SemanticDocuments {
        policy,
        documents,
        availability,
    }
}

fn document(
    policy: DocumentPolicy,
    declaration: &HydratedDeclaration,
    availability: &mut ContentAvailability,
) -> SemanticDocument {
    availability.total += 1;
    let statement = normalize(&declaration.statement_text);
    if !statement.is_empty() {
        availability.with_statement += 1;
    }
    let docstring = declaration.docstring_text.as_deref().map(normalize);
    if docstring.as_deref().is_some_and(|text| !text.is_empty()) {
        availability.with_docstring += 1;
    }
    let body = declaration.definition_body_summary.as_deref().map(normalize);
    if body.as_deref().is_some_and(|text| !text.is_empty()) {
        availability.with_definition_body_summary += 1;
    }

    let text = document_text(policy, declaration, &statement, docstring.as_deref(), body.as_deref());
    SemanticDocument {
        declaration_name: declaration.qualified_name.clone(),
        module_name: declaration.module.clone(),
        declaration_kind: declaration.kind.clone(),
        content_hash: content_hash_for(policy, declaration, &text),
        text,
    }
}

fn document_text(
    policy: DocumentPolicy,
    declaration: &HydratedDeclaration,
    statement: &str,
    docstring: Option<&str>,
    body: Option<&str>,
) -> String {
    match policy {
        DocumentPolicy::Statement => statement.to_owned(),
        DocumentPolicy::NameAndStatement => {
            format!("name: {}\nstatement: {statement}", declaration.qualified_name)
        }
        DocumentPolicy::DefinitionAware => {
            let mut parts = vec![
                format!("name: {}", declaration.qualified_name),
                format!("statement: {statement}"),
            ];
            if let Some(body) = body.filter(|text| !text.is_empty()) {
                parts.push(format!("definition-summary: {body}"));
            }
            parts.join("\n")
        }
        DocumentPolicy::DocstringAugmented => {
            let mut parts = vec![
                format!("name: {}", declaration.qualified_name),
                format!("statement: {statement}"),
            ];
            if let Some(docstring) = docstring.filter(|text| !text.is_empty()) {
                parts.push(format!("docstring: {docstring}"));
            }
            if let Some(body) = body.filter(|text| !text.is_empty()) {
                parts.push(format!("definition-summary: {body}"));
            }
            parts.join("\n")
        }
    }
}

fn content_hash_for(policy: DocumentPolicy, declaration: &HydratedDeclaration, text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(DOCUMENT_POLICY_VERSION.as_bytes());
    hasher.update(policy.id().as_bytes());
    hasher.update(declaration.qualified_name.as_bytes());
    hasher.update(declaration.kind.as_bytes());
    hasher.update(text.as_bytes());
    hex_bytes(&hasher.finalize())
}

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}
