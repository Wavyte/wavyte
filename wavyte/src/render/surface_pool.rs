use crate::compile::plan::{PixelFormat, SurfaceDesc};
use std::collections::HashMap;

/// Pool configuration for cached surfaces.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SurfacePoolOpts {
    /// Maximum bytes retained across all buckets.
    pub(crate) max_pool_bytes: usize,
    /// Maximum number of retained surfaces per (w,h,format) bucket.
    pub(crate) max_surfaces_per_bucket: usize,
}

impl Default for SurfacePoolOpts {
    fn default() -> Self {
        Self {
            // Conservative default. This is a v0.3 internal object for now; Phase 8 will expose
            // this via `SessionOpts`.
            max_pool_bytes: 256 * 1024 * 1024,
            max_surfaces_per_bucket: 8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SurfaceKey {
    w: u32,
    h: u32,
    format: PixelFormat,
}

impl SurfaceKey {
    fn from_desc(desc: SurfaceDesc) -> Self {
        Self {
            w: desc.width,
            h: desc.height,
            format: desc.format,
        }
    }

    fn byte_len(self) -> usize {
        let px = (self.w as usize).saturating_mul(self.h as usize);
        match self.format {
            PixelFormat::Rgba8Premul => px.saturating_mul(4),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct SurfacePoolStats {
    pub(crate) retained_surfaces: usize,
    pub(crate) retained_bytes: usize,
    pub(crate) alloc_surfaces: u64,
    pub(crate) alloc_bytes: u64,
    pub(crate) dropped_on_release: u64,
}

struct Bucket {
    key: SurfaceKey,
    surfaces: Vec<vello_cpu::Pixmap>,
}

/// Bounded pooled allocator for CPU pixmaps used during v0.3 DAG execution.
///
/// Keyed by `(width, height, format)`. Borrow/release must happen at op granularity, not per-pixel.
pub(crate) struct SurfacePool {
    opts: SurfacePoolOpts,
    stats: SurfacePoolStats,

    // Hash lookup is acceptable here: this is op-level, not per-pixel.
    bucket_idx_by_key: HashMap<SurfaceKey, usize>,
    buckets: Vec<Bucket>,
}

impl SurfacePool {
    pub(crate) fn new(opts: SurfacePoolOpts) -> Self {
        Self {
            opts,
            stats: SurfacePoolStats::default(),
            bucket_idx_by_key: HashMap::new(),
            buckets: Vec::new(),
        }
    }

    pub(crate) fn stats(&self) -> SurfacePoolStats {
        self.stats.clone()
    }

    pub(crate) fn borrow(&mut self, desc: SurfaceDesc) -> vello_cpu::Pixmap {
        let key = SurfaceKey::from_desc(desc);
        if let Some(&bi) = self.bucket_idx_by_key.get(&key)
            && let Some(p) = self.buckets[bi].surfaces.pop()
        {
            self.stats.retained_surfaces = self.stats.retained_surfaces.saturating_sub(1);
            self.stats.retained_bytes = self.stats.retained_bytes.saturating_sub(key.byte_len());
            return p;
        }

        self.stats.alloc_surfaces = self.stats.alloc_surfaces.saturating_add(1);
        self.stats.alloc_bytes = self.stats.alloc_bytes.saturating_add(key.byte_len() as u64);

        let w: u16 = key
            .w
            .try_into()
            .unwrap_or_else(|_| panic!("surface width exceeds u16: {}", key.w));
        let h: u16 = key
            .h
            .try_into()
            .unwrap_or_else(|_| panic!("surface height exceeds u16: {}", key.h));
        vello_cpu::Pixmap::new(w, h)
    }

    pub(crate) fn release(&mut self, desc: SurfaceDesc, pixmap: vello_cpu::Pixmap) {
        if self.opts.max_pool_bytes == 0 || self.opts.max_surfaces_per_bucket == 0 {
            self.stats.dropped_on_release = self.stats.dropped_on_release.saturating_add(1);
            return;
        }

        let key = SurfaceKey::from_desc(desc);
        let bytes = key.byte_len();

        if self.stats.retained_bytes.saturating_add(bytes) > self.opts.max_pool_bytes {
            self.stats.dropped_on_release = self.stats.dropped_on_release.saturating_add(1);
            return;
        }

        let bi = match self.bucket_idx_by_key.get(&key).copied() {
            Some(i) => i,
            None => {
                let i = self.buckets.len();
                self.buckets.push(Bucket {
                    key,
                    surfaces: Vec::new(),
                });
                self.bucket_idx_by_key.insert(key, i);
                i
            }
        };

        let bucket = &mut self.buckets[bi];
        if bucket.surfaces.len() >= self.opts.max_surfaces_per_bucket {
            self.stats.dropped_on_release = self.stats.dropped_on_release.saturating_add(1);
            return;
        }

        bucket.surfaces.push(pixmap);
        self.stats.retained_surfaces = self.stats.retained_surfaces.saturating_add(1);
        self.stats.retained_bytes = self.stats.retained_bytes.saturating_add(bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(w: u32, h: u32) -> SurfaceDesc {
        SurfaceDesc {
            width: w,
            height: h,
            format: PixelFormat::Rgba8Premul,
        }
    }

    #[test]
    fn pool_honors_bucket_cap() {
        let mut p = SurfacePool::new(SurfacePoolOpts {
            max_pool_bytes: 1 << 30,
            max_surfaces_per_bucket: 1,
        });
        let d = desc(8, 8);

        let a = p.borrow(d);
        let b = p.borrow(d);
        p.release(d, a);
        p.release(d, b);

        let st = p.stats();
        assert_eq!(st.retained_surfaces, 1);
    }

    #[test]
    fn pool_honors_global_byte_cap() {
        let bytes_8x8 = SurfaceKey::from_desc(desc(8, 8)).byte_len();
        let mut p = SurfacePool::new(SurfacePoolOpts {
            max_pool_bytes: bytes_8x8,
            max_surfaces_per_bucket: 8,
        });
        let d = desc(8, 8);

        let a = p.borrow(d);
        let b = p.borrow(d);
        p.release(d, a);
        p.release(d, b);

        let st = p.stats();
        assert_eq!(st.retained_bytes, bytes_8x8);
        assert_eq!(st.retained_surfaces, 1);
        assert!(st.dropped_on_release >= 1);
    }
}
