// v0.0.4
use bitwise::paging::{pte_encode, pte_is_huge, pte_is_present, pte_phys_addr, pte_flags, vaddr_pt_index};
use crate::{prot::Protection, table::PageTable};

const PAGE_SIZE: u64 = 0x1000;

/// Error returned by [`PageTableWalker::map`] and [`PageTableWalker::map_mmio`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapError {
    /// The buddy allocator could not supply a frame for an intermediate page table.
    OutOfFrames,
    /// `vaddr` or `phys` was not 4 KiB aligned.
    Misaligned,
}

/// Programs the IA32_PAT MSR so PAT index 7 (PWT|PCD|PAT-bit set in a leaf PTE)
/// selects Write-Combining. Indices 0-6 retain Intel firmware defaults.
///
/// Call once during VMM init, before any `MMIO_WC` mapping is installed. The kernel
/// framebuffer should be mapped WC; device register BARs should use `MMIO_UC`.
///
/// # Safety
/// Must be called at CPL=0. Writing an incorrect value to IA32_PAT silently
/// mis-types all subsequent page accesses. Do not call after CPUs have diverged
/// in SMP without coordinating all cores.
pub unsafe fn init_pat() {
    // Intel default IA32_PAT MSR: 0x0007_0406_0007_0406
    //   index: 7     6     5     4     3     2     1     0
    //   type:  UC    UC-   WT    WB    UC    UC-   WT    WB
    // Only PAT7 changes: UC (0x00) -> WC (0x01).
    const PAT_VALUE: u64 =
        0x06u64                 // PAT0: WB  (Write-Back)
        | (0x04u64 <<  8)      // PAT1: WT  (Write-Through)
        | (0x07u64 << 16)      // PAT2: UC- (Uncacheable-)
        | (0x00u64 << 24)      // PAT3: UC  (Uncacheable, strong)
        | (0x06u64 << 32)      // PAT4: WB
        | (0x04u64 << 40)      // PAT5: WT
        | (0x07u64 << 48)      // PAT6: UC-
        | (0x01u64 << 56);     // PAT7: WC  (Write-Combining)  <- changed from UC
    unsafe {
        bitwise::msr::wrmsr(bitwise::msr::msr_num::IA32_PAT, PAT_VALUE);
    }
}

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

    /// Convert a physical address to its HHDM-mapped virtual address.
    #[inline]
    fn phys_to_virt(&self, phys: u64) -> u64 {
        phys + self.hhdm
    }

    /// Return a raw pointer to the PageTable at the given physical frame.
    ///
    /// Creating the pointer is always safe; all callers are responsible for the
    /// unsafe dereference. Two calls with the same `phys` produce aliased raw
    /// pointers — do not create two live `&mut` references from them simultaneously.
    fn table_at_phys(&self, phys: u64) -> *mut PageTable {
        self.phys_to_virt(phys) as *mut PageTable
    }

    /// Allocate a zeroed page frame from the buddy allocator for use as a page table.
    ///
    /// Returns `None` if the allocator is exhausted.
    ///
    /// Contract: `abalone::buddy::alloc_pages` returns HHDM-virtual addresses
    /// because all buddy pages are backed by HHDM mappings established at init time.
    fn alloc_table_frame(&self) -> Option<*mut PageTable> {
        let virt = abalone::buddy::alloc_pages(0)? as u64;
        debug_assert!(virt >= self.hhdm, "buddy returned address below HHDM");
        let table = virt as *mut PageTable;
        unsafe { (*table).zero() };
        Some(table)
    }

    /// Resolve or create the intermediate table entry at `level` for `vaddr`.
    ///
    /// Returns the physical address of the next-level table, or `Err(OutOfFrames)`
    /// if a new frame could not be allocated.
    ///
    /// # Safety
    /// `table_phys` must be the physical address of a live, correctly initialized
    /// page table.
    unsafe fn descend_or_create(&self, table_phys: u64, vaddr: u64, level: u32) -> Result<u64, MapError> {
        let table = self.table_at_phys(table_phys);
        let idx = vaddr_pt_index(vaddr, level) as usize;
        let entry = unsafe { (*table).read(idx) };

        if pte_is_present(entry) {
            Ok(pte_phys_addr(entry, PAGE_SIZE))
        } else {
            let frame_virt = self.alloc_table_frame().ok_or(MapError::OutOfFrames)?;
            let frame_phys = frame_virt as u64 - self.hhdm;
            let new_entry = pte_encode(frame_phys, PAGE_SIZE, pte_flags::PRESENT | pte_flags::WRITABLE);
            unsafe { (*table).write(idx, new_entry) };
            Ok(frame_phys)
        }
    }

    /// Map a single 4 KiB page: virtual `vaddr` to physical `phys` with `prot`.
    ///
    /// Creates intermediate tables as needed. Returns `Ok(Some(old_phys))` when
    /// the virtual address was already mapped (old frame is NOT freed — caller
    /// decides), `Ok(None)` when the address was previously unmapped, or `Err`
    /// on OOM or misalignment.
    ///
    /// The TLB entry for `vaddr` is invalidated on the local CPU after every
    /// successful leaf write.
    ///
    /// # Safety
    /// - `vaddr` and `phys` must each be 4 KiB aligned (returns `Err(Misaligned)` otherwise).
    /// - The kernel page tables must be accessible through the HHDM.
    pub unsafe fn map(&self, vaddr: u64, phys: u64, prot: Protection) -> Result<Option<u64>, MapError> {
        if vaddr & (PAGE_SIZE - 1) != 0 || phys & (PAGE_SIZE - 1) != 0 {
            return Err(MapError::Misaligned);
        }

        let pml4_phys = self.pml4_phys();
        let pdpt_phys = unsafe { self.descend_or_create(pml4_phys, vaddr, 4)? };
        let pd_phys   = unsafe { self.descend_or_create(pdpt_phys,  vaddr, 3)? };
        let pt_phys   = unsafe { self.descend_or_create(pd_phys,    vaddr, 2)? };

        let pt = self.table_at_phys(pt_phys);
        let idx = vaddr_pt_index(vaddr, 1) as usize;
        let old = unsafe { (*pt).read(idx) };
        let old_frame = pte_is_present(old).then(|| pte_phys_addr(old, PAGE_SIZE));
        let leaf = pte_encode(phys, PAGE_SIZE, pte_flags::PRESENT | prot.bits());
        unsafe { (*pt).write(idx, leaf) };
        unsafe { bitwise::instructions::invlpg(vaddr) };
        Ok(old_frame)
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

        let pt = self.table_at_phys(pt_phys);
        let idx = vaddr_pt_index(vaddr, 1) as usize;
        unsafe { (*pt).write(idx, 0) };
        unsafe { bitwise::instructions::invlpg(vaddr) };
    }

    /// Follow an existing intermediate entry; return `None` if not present or huge.
    fn walk_existing(&self, table_phys: u64, vaddr: u64, level: u32) -> Option<u64> {
        let table = self.table_at_phys(table_phys);
        let idx = vaddr_pt_index(vaddr, level) as usize;
        let entry = unsafe { (*table).read(idx) };
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

        let pml4 = self.table_at_phys(pml4_phys);
        let pdpt_entry = unsafe { (*pml4).read(vaddr_pt_index(vaddr, 4) as usize) };
        if !pte_is_present(pdpt_entry) { return None; }
        let pdpt_phys = pte_phys_addr(pdpt_entry, PAGE_SIZE);

        let pdpt = self.table_at_phys(pdpt_phys);
        let pd_entry = unsafe { (*pdpt).read(vaddr_pt_index(vaddr, 3) as usize) };
        if !pte_is_present(pd_entry) { return None; }
        if pte_is_huge(pd_entry) {
            let base = pte_phys_addr(pd_entry, 0x4000_0000);
            return Some(base + (vaddr & 0x3FFF_FFFF));
        }
        let pd_phys = pte_phys_addr(pd_entry, PAGE_SIZE);

        let pd = self.table_at_phys(pd_phys);
        let pt_entry = unsafe { (*pd).read(vaddr_pt_index(vaddr, 2) as usize) };
        if !pte_is_present(pt_entry) { return None; }
        if pte_is_huge(pt_entry) {
            let base = pte_phys_addr(pt_entry, 0x0020_0000);
            return Some(base + (vaddr & 0x001F_FFFF));
        }
        let pt_phys = pte_phys_addr(pt_entry, PAGE_SIZE);

        let pt = self.table_at_phys(pt_phys);
        let leaf = unsafe { (*pt).read(vaddr_pt_index(vaddr, 1) as usize) };
        if !pte_is_present(leaf) { return None; }
        Some(pte_phys_addr(leaf, PAGE_SIZE) + (vaddr & (PAGE_SIZE - 1)))
    }

    /// Map a contiguous MMIO region.
    ///
    /// Maps `size` bytes (rounded up to page boundaries) starting at virtual
    /// `virt_start` to physical `phys_start` with `prot`.
    ///
    /// Use `Protection::MMIO_WC` for framebuffers (requires `init_pat()` called
    /// beforehand). Use `Protection::MMIO_UC` for device register BARs that
    /// require strictly ordered writes.
    ///
    /// # Safety
    /// Same as `map`. Additionally, `phys_start` must refer to a device MMIO
    /// window, not normal RAM.
    pub unsafe fn map_mmio(
        &self,
        virt_start: u64,
        phys_start: u64,
        size:       u64,
        prot:       Protection,
    ) -> Result<(), MapError> {
        let pages = (size + PAGE_SIZE - 1) / PAGE_SIZE;
        for i in 0..pages {
            unsafe {
                self.map(virt_start + i * PAGE_SIZE, phys_start + i * PAGE_SIZE, prot)?;
            }
        }
        Ok(())
    }
}
