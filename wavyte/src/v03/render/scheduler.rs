use crate::v03::compile::plan::{Op, OpId};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Deterministic dependency-count scheduler for executing a v0.3 `RenderPlan` op DAG.
///
/// Determinism rule: when multiple ops are ready, the smallest `OpId` is returned first.
pub(crate) struct DagScheduler {
    indeg: Vec<u32>,
    dependents: Vec<Vec<OpId>>,
    ready: BinaryHeap<Reverse<u32>>,
    remaining: usize,
}

impl DagScheduler {
    pub(crate) fn new(ops: &[Op]) -> Self {
        let n = ops.len();
        let mut indeg = vec![0u32; n];
        let mut dependents = vec![Vec::<OpId>::new(); n];

        for op in ops {
            let oi = op.id.0 as usize;
            let mut count = 0u32;
            for d in &op.deps {
                let di = d.0 as usize;
                dependents[di].push(op.id);
                count = count.saturating_add(1);
            }
            indeg[oi] = count;
        }

        let mut ready = BinaryHeap::<Reverse<u32>>::new();
        for (i, &deg) in indeg.iter().enumerate() {
            if deg == 0 {
                ready.push(Reverse(i as u32));
            }
        }

        Self {
            indeg,
            dependents,
            ready,
            remaining: n,
        }
    }

    pub(crate) fn remaining(&self) -> usize {
        self.remaining
    }

    pub(crate) fn pop_ready(&mut self) -> Option<OpId> {
        let Reverse(id) = self.ready.pop()?;
        Some(OpId(id))
    }

    pub(crate) fn pop_ready_batch(&mut self, max: usize) -> Vec<OpId> {
        let mut out = Vec::with_capacity(max.min(self.ready.len()));
        for _ in 0..max {
            let Some(op) = self.pop_ready() else {
                break;
            };
            out.push(op);
        }
        out
    }

    pub(crate) fn mark_done(&mut self, done: OpId) {
        let di = done.0 as usize;
        self.remaining = self.remaining.saturating_sub(1);

        // Marking an op done is a hot-ish operation, but only at op granularity (not per pixel).
        // Keep the logic tight and avoid allocations here.
        for &dep in &self.dependents[di] {
            let i = dep.0 as usize;
            let d = &mut self.indeg[i];
            *d = d.saturating_sub(1);
            if *d == 0 {
                self.ready.push(Reverse(dep.0));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v03::compile::plan::{OpKind, RenderPlan, SurfaceDesc, SurfaceId};
    use smallvec::smallvec;

    fn op(id: u32, deps: &[u32]) -> Op {
        Op {
            id: OpId(id),
            kind: OpKind::Composite {
                clear_to_transparent: true,
                ops: Box::new(smallvec![]),
            },
            inputs: smallvec![SurfaceId(0)],
            output: SurfaceId(0),
            deps: deps.iter().copied().map(OpId).collect(),
        }
    }

    #[test]
    fn scheduler_is_topological_and_deterministic() {
        // Graph:
        // 0 -> 2
        // 1 -> 2
        // 2 -> 3
        let ops = vec![op(0, &[]), op(1, &[]), op(2, &[0, 1]), op(3, &[2])];
        let _plan = RenderPlan {
            surfaces: vec![SurfaceDesc {
                width: 1,
                height: 1,
                format: crate::v03::compile::plan::PixelFormat::Rgba8Premul,
            }],
            ops: ops.clone(),
            roots: smallvec![SurfaceId(0)],
        };

        let mut sched = DagScheduler::new(&ops);
        let mut out = Vec::<u32>::new();
        while let Some(next) = sched.pop_ready() {
            out.push(next.0);
            sched.mark_done(next);
        }
        assert_eq!(out, vec![0, 1, 2, 3]);
    }
}
