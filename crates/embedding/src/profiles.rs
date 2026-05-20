use fastembed::EmbeddingModel;

use crate::{EmbeddingInputFormat, EmbeddingInputRole, EmbeddingModelSpec, EmbeddingModelSummary, Error, Result};

pub(crate) const BGE_SMALL_PROFILE_ID: &str = "bge-small-en-v1.5";
pub(crate) const BGE_SMALL_MODEL_ID: &str = "BAAI/bge-small-en-v1.5";
pub(crate) const BGE_BASE_PROFILE_ID: &str = "bge-base-en-v1.5";
pub(crate) const BGE_BASE_MODEL_ID: &str = "BAAI/bge-base-en-v1.5";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputRole {
    Document,
    Query,
}

impl InputRole {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Query => "query",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ModelProfile {
    pub(crate) profile_id: &'static str,
    pub(crate) model_id: &'static str,
    pub(crate) dimension: usize,
    pub(crate) max_length: usize,
    pub(crate) input_roles: &'static [InputRole],
    pub(crate) normalized_output: bool,
}

impl ModelProfile {
    pub(crate) fn summary(self, requested: &EmbeddingModelSpec, fingerprint: Option<String>) -> EmbeddingModelSummary {
        EmbeddingModelSummary {
            id: requested.id.clone(),
            revision: requested.revision.clone(),
            fingerprint,
            profile_id: self.profile_id.to_owned(),
            backend_family: "fastembed".to_owned(),
            dimension: self.dimension,
            input_roles: self.input_roles.iter().map(|role| role.label().to_owned()).collect(),
        }
    }

    pub(crate) fn fingerprint_seed(self, requested: &EmbeddingModelSpec) -> String {
        format!(
            "{}:{}:{}:{}:{}:{}:{}:{}",
            self.profile_id,
            self.model_id,
            requested.id,
            requested.revision.as_deref().unwrap_or("default"),
            "fastembed",
            self.dimension,
            self.max_length,
            self.normalized_output
        )
    }

    pub(crate) fn fastembed_model(self) -> Option<EmbeddingModel> {
        match self.profile_id {
            BGE_SMALL_PROFILE_ID => Some(EmbeddingModel::BGESmallENV15),
            BGE_BASE_PROFILE_ID => Some(EmbeddingModel::BGEBaseENV15),
            _ => None,
        }
    }

    pub(crate) fn wrap_text(self, input_format: EmbeddingInputFormat, role: EmbeddingInputRole, text: &str) -> String {
        match (input_format, role) {
            (EmbeddingInputFormat::SymmetricDocument, _) | (_, EmbeddingInputRole::Document) => {
                format!("passage: {text}")
            }
            (EmbeddingInputFormat::AsymmetricQueryDocument, EmbeddingInputRole::Query) => {
                format!("query: {text}")
            }
        }
    }
}

pub(crate) fn resolve_profile(model: &EmbeddingModelSpec) -> Result<ModelProfile> {
    match model.id.as_str() {
        BGE_SMALL_MODEL_ID => Ok(BGE_SMALL_PROFILE),
        BGE_BASE_MODEL_ID => Ok(BGE_BASE_PROFILE),
        _ => Err(Error::UnsupportedModel {
            reason: "unsupported-model-profile".to_owned(),
        }),
    }
}

pub(crate) fn resolve_profile_id(profile_id: &str) -> Result<ModelProfile> {
    match profile_id {
        BGE_SMALL_PROFILE_ID => Ok(BGE_SMALL_PROFILE),
        BGE_BASE_PROFILE_ID => Ok(BGE_BASE_PROFILE),
        _ => Err(Error::UnsupportedModel {
            reason: "unsupported-model-profile".to_owned(),
        }),
    }
}

const DOCUMENT_AND_QUERY: &[InputRole] = &[InputRole::Document, InputRole::Query];

const BGE_SMALL_PROFILE: ModelProfile = ModelProfile {
    profile_id: BGE_SMALL_PROFILE_ID,
    model_id: BGE_SMALL_MODEL_ID,
    dimension: 384,
    max_length: 512,
    input_roles: DOCUMENT_AND_QUERY,
    normalized_output: true,
};

const BGE_BASE_PROFILE: ModelProfile = ModelProfile {
    profile_id: BGE_BASE_PROFILE_ID,
    model_id: BGE_BASE_MODEL_ID,
    dimension: 768,
    max_length: 512,
    input_roles: DOCUMENT_AND_QUERY,
    normalized_output: true,
};
