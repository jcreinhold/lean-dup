//! Conversions between the worker-wire feature row and the shared contract row.
//!
//! The worker capability speaks `lean-dup.worker.v1` (`lean_dup_worker` types,
//! plain `String` keys); the shared semantic-index store speaks the contract DTO
//! (`OpaqueFeatureKey`-typed keys). The two encode the same opaque keys, so these
//! conversions are field-for-field and lossless. They live here so neither the
//! worker wire nor the store schema leaks into the other: the store ingests
//! contract rows, and hydration rebuilds worker-shaped feature facts.

use lean_dup_worker::FeatureRow as WorkerFeatureRow;
use lean_dup_worker::{Fingerprints as WorkerFingerprints, RoleFeature as WorkerRoleFeature};
use lean_semantic_search_contract::{
    DeclarationFeatureRow, Fingerprints as ContractFingerprints, OpaqueFeatureKey, RoleFeature as ContractRoleFeature,
};

/// Translate a worker feature row into the shared store's contract row.
///
/// `source` is left empty: `lean-dup` carries display and source metadata in its
/// own declarations table, not in the shared semantic corpus.
pub(crate) fn feature_to_contract(feature: &WorkerFeatureRow) -> DeclarationFeatureRow {
    DeclarationFeatureRow {
        declaration_id: feature.declaration_id.clone(),
        feature_version: feature.feature_version.clone(),
        fingerprints: ContractFingerprints {
            statement: OpaqueFeatureKey::new(feature.fingerprints.statement.clone()),
            safe_binder_permutation: OpaqueFeatureKey::new(feature.fingerprints.safe_binder_permutation.clone()),
            connective_shape: OpaqueFeatureKey::new(feature.fingerprints.connective_shape.clone()),
            conclusion_shape: OpaqueFeatureKey::new(feature.fingerprints.conclusion_shape.clone()),
        },
        role_features: feature
            .role_features
            .iter()
            .map(|role| ContractRoleFeature {
                role: role.role.clone(),
                key: OpaqueFeatureKey::new(role.key.clone()),
                display: role.display.clone(),
            })
            .collect(),
        binder_count: u32::try_from(feature.binder_count).unwrap_or(u32::MAX),
        low_signal_markers: feature.low_signal_markers.clone(),
        source: None,
    }
}

/// Rebuild the worker-shaped fingerprints a hydrated declaration carries.
pub(crate) fn worker_fingerprints(fingerprints: &ContractFingerprints) -> WorkerFingerprints {
    WorkerFingerprints {
        statement: fingerprints.statement.as_str().to_owned(),
        safe_binder_permutation: fingerprints.safe_binder_permutation.as_str().to_owned(),
        connective_shape: fingerprints.connective_shape.as_str().to_owned(),
        conclusion_shape: fingerprints.conclusion_shape.as_str().to_owned(),
    }
}

/// Rebuild the worker-shaped role features a hydrated declaration carries.
pub(crate) fn worker_role_features(role_features: &[ContractRoleFeature]) -> Vec<WorkerRoleFeature> {
    role_features
        .iter()
        .map(|role| WorkerRoleFeature {
            role: role.role.clone(),
            key: role.key.as_str().to_owned(),
            display: role.display.clone(),
        })
        .collect()
}
