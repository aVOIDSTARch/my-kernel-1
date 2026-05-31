// v0.0.4
use mantle::walker::PageTableWalker;
use spin::Once;

static VMM: Once<PageTableWalker> = Once::new();

/// Initialize the VMM with the HHDM offset provided by Limine.
///
/// Programs the PAT MSR (placing WC at index 7) then constructs the page table
/// walker. Must be called after the heap is up, and before any map/unmap calls.
/// Safe to call exactly once.
pub fn init(hhdm_offset: u64) {
    VMM.call_once(|| {
        unsafe { mantle::walker::init_pat() };
        unsafe { PageTableWalker::new(hhdm_offset) }
    });
}

/// Obtain a reference to the kernel page table walker.
///
/// # Panics
/// Panics if `init` has not been called.
pub fn get() -> &'static PageTableWalker {
    VMM.get().expect("vmm not initialized")
}
