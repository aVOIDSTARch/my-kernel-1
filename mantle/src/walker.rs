// v0.0.2
use bitwise::paging::{pte_encode, pte_is_huge, pte_is_present, pte_phys_addr, pte_flags, vaddr_pt_index};
use crate::{prot::Protection, table::PageTable};

const PAGE_SIZE: u64 = 0x1000;

/// Walks and modifies the kernel's 4-level page table.
///
/// All intermediate table frames are allocated from the buddy allocator and
/// accessed through the HHDM window. Leaf PTEs are always 4 KiB — large/huge
/// pages are detected during `translate` but not created by `map`.
pub struct PageTableWalker {
    hhdm: u64,
}

impl PageTableWalker {
    /// Construct a walker.
    ///
    /// # Safety
    /// `hhdm` must be the physical memory offset reported by Limine. All physical
    /// addresses must be reachable through `phys + hhdm`.
    pub const unsafe fn new(hhdm: u64) -> Self {
        Self { hhdm }
    }

    /// Read CR3 and return the physical base of the active PML4 table.
    #[inline]
    fn pml4_phys(&self) -> u64 {
        let cr3: u64;
        unsafe {
            core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, nomem, preserves_flags));
        }
        cr3 & !0xFFF
    }

    /// Convert a physical address to a virtual (HHDM-mapped) pointer.
    #[inline]
    fn phys_to_virt(&self, phys: u64) -> u64 {
        phys + self.hhdm
    }

    /// Get a mutable reference to a PageTable given its physical address.
    ///
    /// # Safety
    /// `phys` must be a valid, 4 KiB-aligned physical frame.
    unsafe fn table_at_phys(&self, phys: u64) -> &mut PageTable {
        unsafe { &mut *(self.phys_to_virt(phys) as *mut PageTable) }
    }

    /// Allocate a zeroed page frame from the buddy allocator.
    ///
    /// Returns `None` if the allocator is exhausted. The returned pointer is a
    /// HHDM-virtual address; physical = ptr as u64 - hhdm.
    fn alloc_table_frame(&self) -> Option<*mut PageTable> {
        let virt = abalone::buddy::alloc_pages(0)? as u64;
        let table = virt as *mut PageTable;
        unsafe { (*table).zero() };
        Some(table)
    }

    /// Resolve or create the intermediate table entry at `level` for `vaddr`.
    ///
    /// Returns the physical address of the next-level table, or `None` if a
    /// new frame could not be allocated.
    ///
    /// # Safety
    /// `table_phys` must be a valid physical address of a live page table.
    unsafe fn descend_or_create(&self, table_phys: u64, vaddr: u64, level: u32) -> Option<u64> {
        let table = unsafe { self.table_at_phys(table_phys) };
        let idx = vaddr_pt_index(vaddr, level) as usize;
        let entry = table.read(idx);

        if pte_is_present(entry) {
            // Return the physical address of the next level.
            Some(pte_phys_addr(entry, PAGE_SIZE))
        } else {
            // Allocate a new zeroed frame and install it.
            let frame_virt = self.alloc_table_frame()?;
            let frame_phys = frame_virt as u64 - self.hhdm;
            let new_entry = pte_encode(frame_phys, PAGE_SIZE, pte_flags::PRESENT | pte_flags::WRITABLE);
            table.write(idx, new_entry);
            Some(frame_phys)
        }
    }

    /// Map a single 4 KiB page: virtual `vaddr` → physical `phys` with `prot`.
    ///
    /// Creates intermediate tables as needed. If `vaddr` was already mapped,
    /// the existing leaf entry is overwritten and the TLB is invalidated.
    ///
    /// # Safety
    /// - `vaddr` must be 4 KiB aligned.
    /// - `phys` must be 4 KiB aligned.
    /// - The kernel page tables must be set up and accessible through the HHDM.
    pub unsafe fn map(&self, vaddr: u64, phys: u64, prot: Protection) -> Option<()> {
        debug_assert!(vaddr & (PAGE_SIZE - 1) == 0, "vaddr not page-aligned");
        debug_assert!(phys  & (PAGE_SIZE - 1) == 0, "phys not page-aligned");

        let pml4_phys = self.pml4_phys();
        let pdpt_phys = unsafe { self.descend_or_create(pml4_phys, vaddr, 4)? };
        let pd_phys   = unsafe { self.descend_or_create(pdpt_phys,  vaddr, 3)? };
        let pt_phys   = unsafe { self.descend_or_create(pd_phys,    vaddr, 2)? };

        let pt = unsafe { self.table_at_phys(pt_phys) };
        let idx = vaddr_pt_index(vaddr, 1) as usize;
        let leaf = pte_encode(phys, PAGE_SIZE, pte_flags::PRESENT | prot.bits());
        pt.write(idx, leaf);

        unsafe { bitwise::instructions::invlpg(vaddr) };
        Some(())
    }

    /// Unmap a single 4 KiB page at `vaddr`.
    ///
    /// Zeroes the leaf PTE and invalidates the TLB entry. Intermediate tables
    /// are not freed (they may still serve other mappings).
    ///
    /// # Safety
    /// `vaddr` must be 4 KiB aligned and previously mapped by `map`.
    pub unsafe fn unmap(&self, vaddr: u64) {
        let pml4_phys = self.pml4_phys();

        let Some(pdpt_phys) = self.walk_existing(pml4_phys, vaddr, 4) else { return };
        let Some(pd_phys)   = self.walk_existing(pdpt_phys,  vaddr, 3) else { return };
        let Some(pt_phys)   = self.walk_existing(pd_phys,    vaddr, 2) else { return };

        let pt = unsafe { self.table_at_phys(pt_phys) };
        let idx = vaddr_pt_index(vaddr, 1) as usize;
        pt.write(idx, 0);

        unsafe { bitwise::instructions::invlpg(vaddr) };
    }

    /// Follow an existing intermediate entry; return `None` if not present or huge.
    fn walk_existing(&self, table_phys: u64, vaddr: u64, level: u32) -> Option<u64> {
        let table = unsafe { self.table_at_phys(table_phys) };
        let idx = vaddr_pt_index(vaddr, level) as usize;
        let entry = table.read(idx);
        if pte_is_present(entry) && !pte_is_huge(entry) {
            Some(pte_phys_addr(entry, PAGE_SIZE))
        } else {
            None
        }
    }

    /// Translate a virtual address to a physical address.
    ///
    /// Returns `None` if the address is not mapped. Handles 1 GiB and 2 MiB
    /// huge pages transparently.
    pub fn translate(&self, vaddr: u64) -> Option<u64> {
        let pml4_phys = self.pml4_phys();

        // Level 4 → PDPT
        let pml4 = unsafe { self.table_at_phys(pml4_phys) };
        let pdpt_entry = pml4.read(vaddr_pt_index(vaddr, 4) as usize);
        if !pte_is_present(pdpt_entry) { return None; }
        let pdpt_phys = pte_phys_addr(pdpt_entry, PAGE_SIZE);

        // Level 3 → PD  (1 GiB huge page check)
        let pdpt = unsafe { self.table_at_phys(pdpt_phys) };
        let pd_entry = pdpt.read(vaddr_pt_index(vaddr, 3) as usize);
        if !pte_is_present(pd_entry) { return None; }
        if pte_is_huge(pd_entry) {
            let base = pte_phys_addr(pd_entry, 0x4000_0000);
            return Some(base + (vaddr & 0x3FFF_FFFF));
        }
        let pd_phys = pte_phys_addr(pd_entry, PAGE_SIZE);

        // Level 2 → PT  (2 MiB huge page check)
        let pd = unsafe { self.table_at_phys(pd_phys) };
        let pt_entry = pd.read(vaddr_pt_index(vaddr, 2) as usize);
        if !pte_is_present(pt_entry) { return None; }
        if pte_is_huge(pt_entry) {
            let base = pte_phys_addr(pt_entry, 0x0020_0000);
            return Some(base + (vaddr & 0x001F_FFFF));
        }
        let pt_phys = pte_phys_addr(pt_entry, PAGE_SIZE);

        // Level 1 — leaf PTE
        let pt = unsafe { self.table_at_phys(pt_phys) };
        let leaf = pt.read(vaddr_pt_index(vaddr, 1) as usize);
        if !pte_is_present(leaf) { return None; }
        Some(pte_phys_addr(leaf, PAGE_SIZE) + (vaddr & (PAGE_SIZE - 1)))
    }

    /// Map a contiguous MMIO region.
    ///
    /// Maps `size` bytes (rounded up to page boundaries) starting at virtual
    /// `virt_start` to physical `phys_start` with `Protection::MMIO`.
    ///
    /// # Safety
    /// Same as `map`. Additionally, `phys_start` must refer to a device MMIO
    /// window, not normal RAM.
    pub unsafe fn map_mmio(&self, virt_start: u64, phys_start: u64, size: u64) -> Option<()> {
        let pages = (size + PAGE_SIZE - 1) / PAGE_SIZE;
        for i in 0..pages {
            unsafe {
                self.map(virt_start + i * PAGE_SIZE, phys_start + i * PAGE_SIZE, Protection::MMIO)?;
            }
        }
        Some(())
    }
}
