// v0.0.2
/// Base page size for x86_64.
pub const PAGE_SIZE: usize = 4096;

/// Buddy allocator maximum order.
/// 2^17 pages x 4 KiB = 512 MiB addressable per buddy instance.
/// Bitmap cost: ~136 KiB BSS.
pub const BUDDY_MAX_ORDER: usize = 17;
