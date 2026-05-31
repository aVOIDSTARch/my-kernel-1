// v0.0.3
use abalone::buddy::BUDDY;
use crate::memory::vmm;

/// Stack allocation result.
pub struct KernelStack {
    /// Virtual address of the top of the stack (initial RSP value).
    pub top:       u64,
    /// Virtual address of the guard page (the page below the stack base).
    /// This page is unmapped; writing to it produces a #PF.
    pub guard_virt: u64,
}

/// Allocate a kernel stack of at least `min_pages` pages with a guard page.
///
/// Allocates `1 << order` pages where `order` is the smallest value such that
/// `(1 << order) > min_pages`. The first page is left unmapped as a guard;
/// the remaining pages form the stack.
///
/// Returns the virtual stack top (initial RSP) and the guard page address.
///
/// # Safety
/// Buddy and VMM must both be initialized.
pub unsafe fn alloc_kernel_stack(min_pages: usize) -> KernelStack {
    // Round up to next power of two so the entire block is one buddy allocation.
    // min_pages=8 → order=4 (16 pages): 1 guard + 15 stack.
    let total_order = (min_pages + 1).next_power_of_two().trailing_zeros() as usize;
    let total_pages = 1usize << total_order;

    let base_virt = {
        let mut buddy = BUDDY.lock();
        buddy.alloc_pages(total_order).expect("kernel stack OOM") as u64
    };

    // Unmap the guard page (bottom of allocation).
    // Safety: base_virt is HHDM-mapped by Limine; unmapping removes the
    // Limine leaf PTE. Any write below the stack top will now #PF.
    unsafe {
        vmm::get().unmap(base_virt);
    }

    let guard_virt = base_virt;
    let stack_top  = base_virt + (total_pages as u64) * 0x1000;

    KernelStack { top: stack_top, guard_virt }
}
