use fastembed::EmbeddingModel;

use crate::{EmbeddingModelFileRole, EmbeddingModelSpec, EmbeddingModelSummary, Error, RequiredModelFile, Result};

pub(crate) const BGE_SMALL_PROFILE_ID: &str = "bge-small-en-v1.5";
pub(crate) const BGE_SMALL_MODEL_ID: &str = "BAAI/bge-small-en-v1.5";
pub(crate) const LEGACY_MINILM_PROFILE_ID: &str = "legacy-minilm-rerank-baseline";
pub(crate) const LEGACY_MINILM_MODEL_ID: &str = "sentence-transformers/all-MiniLM-L6-v2";
pub(crate) const QWEN3_MODEL_ID: &str = "Qwen/Qwen3-Embedding-0.6B";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendFamily {
    FastEmbed,
    LegacyCandleBert,
}

impl BackendFamily {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::FastEmbed => "fastembed",
            Self::LegacyCandleBert => "legacy-candle-bert",
        }
    }
}

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
    pub(crate) backend: BackendFamily,
    pub(crate) dimension: usize,
    pub(crate) max_length: usize,
    pub(crate) input_roles: &'static [InputRole],
    pub(crate) normalized_output: bool,
    pub(crate) support_status: ProfileSupportStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfileSupportStatus {
    Supported,
    UnsupportedNotEnabled,
}

impl ModelProfile {
    pub(crate) fn summary(self, requested: &EmbeddingModelSpec, fingerprint: Option<String>) -> EmbeddingModelSummary {
        EmbeddingModelSummary {
            id: requested.id.clone(),
            revision: requested.revision.clone(),
            fingerprint,
            profile_id: self.profile_id.to_owned(),
            backend_family: self.backend.label().to_owned(),
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
            self.backend.label(),
            self.dimension,
            self.max_length,
            self.normalized_output
        )
    }

    pub(crate) fn required_files(self) -> &'static [RequiredModelFile] {
        match self.backend {
            BackendFamily::FastEmbed => BGE_SMALL_REQUIRED_FILES,
            BackendFamily::LegacyCandleBert => LEGACY_MINILM_REQUIRED_FILES,
        }
    }

    pub(crate) fn fastembed_model(self) -> Option<EmbeddingModel> {
        match self.profile_id {
            BGE_SMALL_PROFILE_ID => Some(EmbeddingModel::BGESmallENV15),
            _ => None,
        }
    }

    pub(crate) fn wrap_document(self, text: &str) -> String {
        match self.profile_id {
            BGE_SMALL_PROFILE_ID => format!("passage: {text}"),
            _ => text.to_owned(),
        }
    }
}

pub(crate) fn resolve_profile(model: &EmbeddingModelSpec) -> Result<ModelProfile> {
    match model.id.as_str() {
        BGE_SMALL_MODEL_ID => Ok(BGE_SMALL_PROFILE),
        LEGACY_MINILM_MODEL_ID => Ok(LEGACY_MINILM_PROFILE),
        QWEN3_MODEL_ID => Ok(QWEN3_PROFILE),
        _ => Err(Error::UnsupportedModel {
            reason: "unsupported-model-profile".to_owned(),
        }),
    }
}

const DOCUMENT_AND_QUERY: &[InputRole] = &[InputRole::Document, InputRole::Query];
const DOCUMENT_ONLY: &[InputRole] = &[InputRole::Document];

const BGE_SMALL_PROFILE: ModelProfile = ModelProfile {
    profile_id: BGE_SMALL_PROFILE_ID,
    model_id: BGE_SMALL_MODEL_ID,
    backend: BackendFamily::FastEmbed,
    dimension: 384,
    max_length: 512,
    input_roles: DOCUMENT_AND_QUERY,
    normalized_output: true,
    support_status: ProfileSupportStatus::Supported,
};

const BGE_SMALL_REQUIRED_FILES: &[RequiredModelFile] = &[
    RequiredModelFile {
        role: EmbeddingModelFileRole::RuntimeModel,
        filename: "onnx/model.onnx",
    },
    RequiredModelFile {
        role: EmbeddingModelFileRole::Config,
        filename: "config.json",
    },
    RequiredModelFile {
        role: EmbeddingModelFileRole::Tokenizer,
        filename: "tokenizer.json",
    },
    RequiredModelFile {
        role: EmbeddingModelFileRole::TokenizerConfig,
        filename: "tokenizer_config.json",
    },
    RequiredModelFile {
        role: EmbeddingModelFileRole::SpecialTokens,
        filename: "special_tokens_map.json",
    },
];

const LEGACY_MINILM_PROFILE: ModelProfile = ModelProfile {
    profile_id: LEGACY_MINILM_PROFILE_ID,
    model_id: LEGACY_MINILM_MODEL_ID,
    backend: BackendFamily::LegacyCandleBert,
    dimension: 384,
    max_length: 256,
    input_roles: DOCUMENT_ONLY,
    normalized_output: true,
    support_status: ProfileSupportStatus::Supported,
};

const QWEN3_PROFILE: ModelProfile = ModelProfile {
    profile_id: "qwen3-embedding-0.6b",
    model_id: QWEN3_MODEL_ID,
    backend: BackendFamily::FastEmbed,
    dimension: 1024,
    max_length: 8192,
    input_roles: DOCUMENT_AND_QUERY,
    normalized_output: true,
    support_status: ProfileSupportStatus::UnsupportedNotEnabled,
};

const LEGACY_MINILM_REQUIRED_FILES: &[RequiredModelFile] = &[
    RequiredModelFile {
        role: EmbeddingModelFileRole::Config,
        filename: "config.json",
    },
    RequiredModelFile {
        role: EmbeddingModelFileRole::Tokenizer,
        filename: "tokenizer.json",
    },
    RequiredModelFile {
        role: EmbeddingModelFileRole::TokenizerConfig,
        filename: "tokenizer_config.json",
    },
    RequiredModelFile {
        role: EmbeddingModelFileRole::SpecialTokens,
        filename: "special_tokens_map.json",
    },
    RequiredModelFile {
        role: EmbeddingModelFileRole::PoolingConfig,
        filename: "1_Pooling/config.json",
    },
    RequiredModelFile {
        role: EmbeddingModelFileRole::Weights,
        filename: "model.safetensors",
    },
];
