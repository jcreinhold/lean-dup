//! Lake workspace and mathlib environment resolution.
//!
//! This crate owns project identity: selected module roots, Lake manifests,
//! Lean toolchains, mathlib source roots, and execution roots. It hides the
//! probing and canonicalization rules from indexing, search, and CLI callers.

mod error;
mod mathlib;
mod workspace;

pub use error::{Error, Result};
pub use mathlib::{
    ProjectMathlib, resolve_for_workspace as resolve_workspace_mathlib, resolve_project as resolve_project_mathlib,
};
pub use workspace::{ResolvedWorkspace, SourceFile, WorkspaceRequest, resolve};
