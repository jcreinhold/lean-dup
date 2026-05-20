use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{EmbeddingInputFormat, EmbeddingInputPolicy, Error, Result, hex_bytes};

#[derive(Debug, Clone)]
pub(crate) struct VectorCache {
    root: PathBuf,
    model_fingerprint: String,
    input_format_id: String,
    input_format_version: String,
    policy_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedVector {
    values: Vec<f32>,
}

impl VectorCache {
    pub(crate) fn new(
        root: Option<PathBuf>,
        model_fingerprint: String,
        input_format: EmbeddingInputFormat,
        input_policy: &EmbeddingInputPolicy,
    ) -> Self {
        Self {
            root: root.unwrap_or_else(default_vector_cache_root),
            model_fingerprint,
            input_format_id: input_format.id().to_owned(),
            input_format_version: input_format.version().to_owned(),
            policy_version: input_policy.version.clone(),
        }
    }

    pub(crate) fn get(&self, text: &str) -> Result<Option<Vec<f32>>> {
        let path = self.path_for(text);
        if !path.exists() {
            return Ok(None);
        }
        let json = fs::read_to_string(path).map_err(|source| Error::VectorCache {
            operation: "read",
            source,
        })?;
        let cached: CachedVector = serde_json::from_str(&json).map_err(|source| Error::Json {
            artifact: "embedding-vector-cache",
            source,
        })?;
        Ok(Some(cached.values))
    }

    pub(crate) fn put(&self, text: &str, values: &[f32]) -> Result<()> {
        let path = self.path_for(text);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| Error::VectorCache {
                operation: "create-dir",
                source,
            })?;
        }
        let json = serde_json::to_string(&CachedVector {
            values: values.to_vec(),
        })
        .map_err(|source| Error::Json {
            artifact: "embedding-vector-cache",
            source,
        })?;
        fs::write(path, json).map_err(|source| Error::VectorCache {
            operation: "write",
            source,
        })
    }

    pub(crate) fn key_for(&self, text: &str) -> String {
        cache_key(
            &self.model_fingerprint,
            &self.input_format_id,
            &self.input_format_version,
            &self.policy_version,
            text,
        )
    }

    fn path_for(&self, text: &str) -> PathBuf {
        self.root
            .join(&self.model_fingerprint)
            .join(format!("{}.json", self.key_for(text)))
    }
}

fn default_vector_cache_root() -> PathBuf {
    PathBuf::from("target").join("search-quality").join("embedding-cache")
}

fn cache_key(
    model_fingerprint: &str,
    input_format_id: &str,
    input_format_version: &str,
    policy_version: &str,
    text: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(model_fingerprint.as_bytes());
    hasher.update([0]);
    hasher.update(input_format_id.as_bytes());
    hasher.update([0]);
    hasher.update(input_format_version.as_bytes());
    hasher.update([0]);
    hasher.update(policy_version.as_bytes());
    hasher.update([0]);
    hasher.update(text.as_bytes());
    hex_bytes(&hasher.finalize())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn cache_key_changes_with_model_format_policy_or_text() {
        let key = cache_key("model-a", "format-a", "format-v1", "policy-a", "text-a");
        assert_ne!(key, cache_key("model-b", "format-a", "format-v1", "policy-a", "text-a"));
        assert_ne!(key, cache_key("model-a", "format-b", "format-v1", "policy-a", "text-a"));
        assert_ne!(key, cache_key("model-a", "format-a", "format-v2", "policy-a", "text-a"));
        assert_ne!(key, cache_key("model-a", "format-a", "format-v1", "policy-b", "text-a"));
        assert_ne!(key, cache_key("model-a", "format-a", "format-v1", "policy-a", "text-b"));
    }

    #[test]
    fn vector_cache_roundtrip_preserves_values() -> Result<()> {
        let temp = TempDir::new().map_err(|source| Error::VectorCache {
            operation: "tempdir",
            source,
        })?;
        let cache = VectorCache::new(
            Some(temp.path().to_path_buf()),
            "model".to_owned(),
            EmbeddingInputFormat::default(),
            &EmbeddingInputPolicy::default(),
        );
        assert!(cache.get("input")?.is_none());
        cache.put("input", &[0.1, 0.2])?;
        assert_eq!(cache.get("input")?, Some(vec![0.1, 0.2]));
        Ok(())
    }
}
