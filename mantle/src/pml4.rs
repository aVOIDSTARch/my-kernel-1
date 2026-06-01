// v0.0.2

use crate::{prot::Protection, table::PageTable};
use bitwise::paging::{pte_encode, pte_flags, vaddr_pt_index};
use abalone::buddy::BUDDY;

const PAGE_SIZE:   u64 = 0x1000;
const HUGE_2M:     u64 = 0x0020_0000;
const HUGE_1G:     u64 = 0x4000_0000;

/// Build a new PML4 covering all regions the kernel needs, then load it
/// into CR3, atomically replacing Limine's page tables.
///
/// After this returns, CR3 points to kernel-owned frames. All frames used
/// for the new page tables were sourced from the buddy (`Usable` memory).
///
/// # Safety
/// - Buddy and VMM must be initialized.
/// - Interrupts should be disabled for the duration (or IDT must be valid
///   under both old and new page tables, which it is since both map the
///   kernel image identically).
/// - Must not be called while running on a stack outside the HHDM.
pub unsafe fn install_kernel_pml4(
    hhdm:              u64,
    kernel_virt_start: u64,
    kernel_virt_end:   u64,
    kernel_phys_start: u64,
    phys_mem_size:     u64,   // upper bound of physical memory to HHDM-map
    fb_virt:           u64,
    fb_phys:           u64,
    fb_pages:          u64,
) {
    // Allocate root PML4 frame from buddy.
    let pml4_phys = alloc_zero_frame(hhdm);

    // 1. Map HHDM using 2 MiB huge pages.
    map_hhdm_2m(hhdm, pml4_phys, phys_mem_size);

    // 2. Map kernel image (4 KiB pages, correct protection per section).
    //    For now, map the entire image RWX — split by section in a later pass.
    map_range_4k(hhdm, pml4_phys,
        kernel_virt_start, kernel_phys_start,
        (kernel_virt_end - kernel_virt_start + PAGE_SIZE - 1) / PAGE_SIZE,
        Protection::KERNEL_RWX_BOOT);

    // 3. Map framebuffer MMIO (WC, already mapped under old tables).
    map_range_4k(hhdm, pml4_phys,
        fb_virt, fb_phys, fb_pages, Protection::MMIO_WC);

    // 4. Write CR3 — atomic from the CPU's perspective; TLB is flushed.
    unsafe {
        core::arch::asm!(
            "mov cr3, {pml4}",
            pml4 = in(reg) pml4_phys,
            options(nostack, preserves_flags),
        );
    }
    // Execution continues on the new page tables. Limine's PML4 frames are
    // now unreferenced and safe to free via release().
}
