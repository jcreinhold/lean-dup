//! Lake workspace and mathlib environment resolution.
//!
//! This crate owns project identity: selected module roots, Lake manifests,
//! Lean toolchains, mathlib source roots, and execution roots. It hides the
//! probing and canonicalization rules from indexing, search, and CLI callers.

mod error;
pub mod mathlib;
pub mod workspace;

pub use error::{Error, Result};
pub use workspace::{ResolvedWorkspace, WorkspaceRequest, resolve};
