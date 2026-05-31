// v0.0.2
use mantle::walker::PageTableWalker;
use spin::Once;

static VMM: Once<PageTableWalker> = Once::new();

/// Initialise the VMM with the HHDM offset provided by Limine.
///
/// Must be called after the heap is up (buddy allocator seeded), and before
/// any `map`/`unmap`/`translate` calls. Safe to call exactly once.
pub fn init(hhdm_offset: u64) {
    VMM.call_once(|| unsafe { PageTableWalker::new(hhdm_offset) });
}

/// Obtain a reference to the kernel page table walker.
///
/// # Panics
/// Panics if `init` has not been called.
pub fn get() -> &'static PageTableWalker {
    VMM.get().expect("vmm not initialized")
}
