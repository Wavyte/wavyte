#![allow(dead_code, unused_imports)]

#[cfg(feature = "alloc-track")]
mod imp {
    use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
    use std::alloc::System;

    #[global_allocator]
    static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

    #[derive(Debug, Clone, Copy, Default)]
    pub(crate) struct AllocStats {
        pub(crate) allocations: usize,
        pub(crate) deallocations: usize,
        pub(crate) reallocations: usize,
        pub(crate) bytes_allocated: usize,
        pub(crate) bytes_deallocated: usize,
        pub(crate) bytes_reallocated: isize,
    }

    impl From<stats_alloc::Stats> for AllocStats {
        fn from(s: stats_alloc::Stats) -> Self {
            Self {
                allocations: s.allocations,
                deallocations: s.deallocations,
                reallocations: s.reallocations,
                bytes_allocated: s.bytes_allocated,
                bytes_deallocated: s.bytes_deallocated,
                bytes_reallocated: s.bytes_reallocated,
            }
        }
    }

    pub(crate) struct AllocRegion {
        region: Region<'static, System>,
    }

    impl AllocRegion {
        pub(crate) fn new() -> Self {
            Self {
                region: Region::new(GLOBAL),
            }
        }

        pub(crate) fn change(&self) -> AllocStats {
            self.region.change().into()
        }
    }
}

#[cfg(not(feature = "alloc-track"))]
mod imp {
    #[derive(Debug, Clone, Copy, Default)]
    pub(crate) struct AllocStats {
        pub(crate) allocations: usize,
        pub(crate) deallocations: usize,
        pub(crate) reallocations: usize,
        pub(crate) bytes_allocated: usize,
        pub(crate) bytes_deallocated: usize,
        pub(crate) bytes_reallocated: isize,
    }

    pub(crate) struct AllocRegion;

    impl AllocRegion {
        pub(crate) fn new() -> Self {
            Self
        }

        pub(crate) fn change(&self) -> AllocStats {
            AllocStats::default()
        }
    }
}

pub(crate) use imp::{AllocRegion, AllocStats};
