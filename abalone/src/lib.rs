// v0.0.4
//! abalone -- bare-metal x86_64 allocator library for the crusty_os workspace.
//!
//! Provides the full allocator stack used by `crusty_os`:
//!
//! ```text
//! GlobalAlloc  (Box / Vec / alloc::*)
//!   └── TlsfAllocator  -- O(1) alloc/dealloc, sub-page granularity
//!         └── BuddyAllocator -- page-granularity, binary buddy system
//! ```
//!
//! [`SlabCache<T>`] provides a typed per-object cache on top of the buddy
//! allocator for high-frequency fixed-size allocations.

#![no_std]

pub mod buddy;
pub mod slab;
pub mod tlsf;

// ── Constants (internal; canonical kernel copies live in src/memory/config.rs) ─

pub(crate) const PAGE_SIZE: usize = 4096;
pub(crate) const BUDDY_MAX_ORDER: usize = 17; // 2^17 pages x 4 KiB = 512 MiB; keeps BSS ~136 KiB

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AllocStats {
    pub total_bytes:   u64,
    pub used_bytes:    u64,
    pub free_bytes:    u64,
    pub alloc_count:   u64,
    pub dealloc_count: u64,
    pub peak_bytes:    u64,
}
