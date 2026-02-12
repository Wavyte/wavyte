//! v0.3 render backend implementation (CPU-first).
//!
//! Phase 6 introduces a pooled CPU backend that executes the v0.3 `RenderPlan` DAG.

/// Backend trait and frame type(s) for v0.3 rendering.
pub mod backend;
/// CPU backend implementation for v0.3.
pub mod cpu;
pub(crate) mod scheduler;
pub(crate) mod surface_pool;
