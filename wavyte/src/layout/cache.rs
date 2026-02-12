/// Layout per-node cache used to avoid unnecessary layout solves.
#[derive(Debug, Default)]
pub(crate) struct LayoutCache {
    /// Stable per-frame hash of the effective Taffy style inputs for each node.
    ///
    /// `u64::MAX` is used as the initial sentinel so the first evaluated frame is always dirty.
    pub(crate) style_hash_by_node: Vec<u64>,
}

impl LayoutCache {
    pub(crate) fn reset(&mut self, n: usize) {
        self.style_hash_by_node.clear();
        self.style_hash_by_node.resize(n, u64::MAX);
    }

    pub(crate) fn ensure_len(&mut self, n: usize) {
        if self.style_hash_by_node.len() != n {
            self.reset(n);
        }
    }
}
