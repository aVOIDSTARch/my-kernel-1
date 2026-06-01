// v0.1.1
use crate::{prot::Protection, table::PageTable};
use bitwise::paging::{
    pte_encode, pte_flags, pte_is_present, pte_phys_addr, vaddr_pt_index,
};

const PAGE_SIZE: u64 = 0x1000;
const HUGE_2M:   u64 = 0x0020_0000;

/// Build a new PML4 covering all regions the kernel needs, then load it
/// into CR3, atomically replacing Limine's page tables.
///
/// After this returns, CR3 points to kernel-owned frames sourced entirely
/// from the buddy (`Usable` memory). Limine's page-table frames may be
/// freed to the buddy once this returns.
///
/// Mappings installed:
/// - HHDM (`hhdm..hhdm+phys_mem_size`): 2 MiB huge pages, WB, NX, global.
///   Covers the kernel stack, buddy metadata, TLSF heap, and framebuffer.
/// - Kernel image: 4 KiB pages, RWX (boot-time W^X exception), global.
///
/// The framebuffer is not mapped separately: its virtual address is in the
/// HHDM range and covered by the 2 MiB mapping above. It operates with WB
/// caching until a later pass installs a dedicated WC mapping.
///
/// # Safety
/// - Buddy must be initialized and hold enough `Usable` frames for the
///   page tables (~258 frames for a 512 GiB HHDM + kernel image PT).
/// - Must be called with interrupts disabled, or with an IDT that is valid
///   under both old and new page tables (it is, since both map the kernel image).
/// - RSP must point into the HHDM at call time.
pub unsafe fn install_kernel_pml4(
    hhdm:              u64,
    kernel_virt_start: u64,
    kernel_virt_end:   u64,
    kernel_phys_start: u64,
    phys_mem_size:     u64,
) {
    let pml4_phys = alloc_zero_frame(hhdm);

    // 1. Map all physical memory via the HHDM window using 2 MiB huge pages.
    //    This covers the kernel stack, buddy metadata, TLSF heap, and framebuffer.
    map_hhdm_2m(hhdm, pml4_phys, phys_mem_size);

    // 2. Map the kernel image at its higher-half virtual address with 4 KiB pages.
    //    The image virtual address is outside the HHDM, using a different PML4 entry.
    let kernel_pages = (kernel_virt_end - kernel_virt_start + PAGE_SIZE - 1) / PAGE_SIZE;
    map_range_4k(
        hhdm, pml4_phys,
        kernel_virt_start, kernel_phys_start,
        kernel_pages,
        Protection::KERNEL_RWX_BOOT,
    );

    // 3. Write CR3. TLB flushed on local CPU (global entries survive — intentional).
    unsafe {
        core::arch::asm!(
            "mov cr3, {pml4}",
            pml4 = in(reg) pml4_phys,
            options(nostack, preserves_flags),
        );
    }
    // Execution continues under the new page tables. Limine's PML4 frames are
    // now unreferenced and safe to return to the buddy via boot.release().
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Allocate one order-0 frame from the buddy and return its physical address.
/// Zeroes the frame so all PTEs start as Not Present.
/// Panics on OOM — unrecoverable during early boot page table construction.
fn alloc_zero_frame(hhdm: u64) -> u64 {
    let virt = abalone::buddy::alloc_pages(0)
        .expect("pml4: buddy OOM during page table construction") as u64;
    let table = virt as *mut PageTable;
    unsafe { (*table).zero() };
    virt - hhdm
}

/// Resolve or create the intermediate page table at `level` for `vaddr`.
/// Returns the physical address of the next-level table. Panics on OOM.
///
/// # Safety
/// `table_phys` must be the physical address of a valid, initialized page table.
unsafe fn descend_or_create(hhdm: u64, table_phys: u64, vaddr: u64, level: u32) -> u64 {
    let table = (hhdm + table_phys) as *mut PageTable;
    let idx   = vaddr_pt_index(vaddr, level) as usize;
    let entry = unsafe { (*table).read(idx) };
    if pte_is_present(entry) {
        pte_phys_addr(entry, PAGE_SIZE)
    } else {
        let child_phys = alloc_zero_frame(hhdm);
        let e = pte_encode(child_phys, PAGE_SIZE, pte_flags::PRESENT | pte_flags::WRITABLE);
        unsafe { (*table).write(idx, e) };
        child_phys
    }
}

/// Map `phys_mem_size` bytes of physical memory as HHDM using 2 MiB huge pages.
/// Pages are global, writable, no-execute — suitable for kernel data and stacks.
fn map_hhdm_2m(hhdm: u64, pml4_phys: u64, phys_mem_size: u64) {
    let huge_pages = (phys_mem_size + HUGE_2M - 1) / HUGE_2M;
    for i in 0..huge_pages {
        let phys = i * HUGE_2M;
        let virt = hhdm + phys;
        let pdpt_phys = unsafe { descend_or_create(hhdm, pml4_phys, virt, 4) };
        let pd_phys   = unsafe { descend_or_create(hhdm, pdpt_phys, virt, 3) };
        let pd  = (hhdm + pd_phys) as *mut PageTable;
        let idx = vaddr_pt_index(virt, 2) as usize;
        let pde = pte_encode(
            phys, HUGE_2M,
            pte_flags::PRESENT | pte_flags::WRITABLE
                | pte_flags::NO_EXECUTE | pte_flags::HUGE_PAGE | pte_flags::GLOBAL,
        );
        unsafe { (*pd).write(idx, pde) };
    }
}

/// Map `pages` contiguous 4 KiB pages: `virt_start..+pages*4K -> phys_start..+pages*4K`.
fn map_range_4k(
    hhdm:       u64,
    pml4_phys:  u64,
    virt_start: u64,
    phys_start: u64,
    pages:      u64,
    prot:       Protection,
) {
    for i in 0..pages {
        let virt = virt_start + i * PAGE_SIZE;
        let phys = phys_start + i * PAGE_SIZE;
        let pdpt_phys = unsafe { descend_or_create(hhdm, pml4_phys, virt, 4) };
        let pd_phys   = unsafe { descend_or_create(hhdm, pdpt_phys, virt, 3) };
        let pt_phys   = unsafe { descend_or_create(hhdm, pd_phys,   virt, 2) };
        let pt  = (hhdm + pt_phys) as *mut PageTable;
        let idx = vaddr_pt_index(virt, 1) as usize;
        let leaf = pte_encode(phys, PAGE_SIZE, pte_flags::PRESENT | prot.bits());
        unsafe { (*pt).write(idx, leaf) };
    }
}
