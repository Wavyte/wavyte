//! v0.3 render backend implementation (CPU-first).
//!
//! Phase 6 introduces a pooled CPU backend that executes the v0.3 `RenderPlan` DAG.

pub(crate) mod surface_pool;
