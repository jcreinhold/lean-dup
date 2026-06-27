//! Lean worker process boundary.
//!
//! This crate owns worker capability access, worker version policy, and
//! request/response data exchanged with Lean. Callers should not know the
//! transport framing, process lifecycle, request envelopes, or timeout
//! mechanics. Public row, progress, and diagnostic DTOs are stable worker
//! capability facts after transport details have already been hidden.

pub mod toolchain;
mod worker;

pub use worker::{
    DeclarationRow, ExtractBatch, FeatureRow, FeaturesBatch, Fingerprints, IndexBatch, IndexStreamItem,
    ModuleDescriptor, ProbeBatch, ProbePair, ProbeResult, RoleFeature, SourcePoint, SourceSpan, WorkerCall,
    WorkerClient, WorkerDiagnostic, WorkerError, WorkerEvent, WorkerIdentity, WorkerSubstrateFacts, WorkerVersion,
};

mod perf {
    #[derive(Debug, Clone, Copy)]
    pub enum CostClass {
        LeanSemantic,
    }

    pub fn record_count(_class: CostClass, _name: impl Into<String>, _count: u64) {}
}
