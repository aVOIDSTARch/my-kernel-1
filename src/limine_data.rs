// v0.0.11
//! Limine boot protocol data harvesting.
//!
//! This module is the **sole** point of contact between the kernel and Limine.
//! All Limine request statics are private to this file. Nothing outside this
//! module can dereference a Limine response pointer.
//!
//! # Usage sequence
//!
//! ```rust
//! let boot = unsafe { LimineData::harvest() };
//!
//! gdt::init();
//! interrupts::init();
//!
//! memory::heap::init(boot.regions(), boot.kernel_phys_start,
//!                    boot.kernel_phys_end, boot.hhdm_offset);
//!
//! memory::vmm::init(boot.hhdm_offset);
//!
//! // All Limine data is now in `boot`. Release reclaimable pages:
//! unsafe { boot.release() };
//! ```

#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, Ordering};

use limine::{
    memmap::{
        MEMMAP_ACPI_NVS, MEMMAP_ACPI_RECLAIMABLE, MEMMAP_BAD_MEMORY,
        MEMMAP_BOOTLOADER_RECLAIMABLE, MEMMAP_EXECUTABLE_AND_MODULES,
        MEMMAP_FRAMEBUFFER, MEMMAP_USABLE,
    },
    request::{
        BootloaderInfoRequest, ExecutableAddressRequest, FramebufferRequest, HhdmRequest,
        MemmapRequest, RsdpRequest,
    },
    BaseRevision, RequestsEndMarker, RequestsStartMarker,
};

use abalone::buddy::BUDDY;
use bitwise::align::{align_down, align_up};

// ── Limine protocol anchors (private) ────────────────────────────────────────
// Nothing outside this file may name these statics.

#[used]
#[unsafe(link_section = ".limine_requests_start")]
static REQUESTS_START: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[unsafe(link_section = ".limine_requests_start")]
static BASE_REVISION: BaseRevision = BaseRevision::with_revision(2);

#[used]
#[unsafe(link_section = ".limine_requests")]
static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
static MEMMAP_REQUEST: MemmapRequest = MemmapRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
static KERNEL_ADDRESS_REQUEST: ExecutableAddressRequest = ExecutableAddressRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
static RSDP_REQUEST: RsdpRequest = RsdpRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
static BOOTLOADER_INFO_REQUEST: BootloaderInfoRequest = BootloaderInfoRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests_end")]
static REQUESTS_END: RequestsEndMarker = RequestsEndMarker::new();

// ── Harvest guard ─────────────────────────────────────────────────────────────

static HARVESTED: AtomicBool = AtomicBool::new(false);

// ── Public data types ─────────────────────────────────────────────────────────

/// A memory map region copied out of the Limine response as plain values.
/// No pointer into Limine memory is retained.
#[derive(Debug, Clone, Copy)]
pub struct MemoryRegion {
    pub base:        u64,
    pub length:      u64,
    pub region_type: MemoryRegionType,
}

impl MemoryRegion {
    #[inline] pub fn end(&self) -> u64 { self.base + self.length }

    /// Page-aligned base (rounded up).
    #[inline] pub fn aligned_base(&self) -> u64 { align_up(self.base, 4096) }

    /// Page-aligned end (rounded down).
    #[inline] pub fn aligned_end(&self) -> u64 { align_down(self.end(), 4096) }

    /// True if at least one full page fits after alignment.
    #[inline] pub fn has_pages(&self) -> bool { self.aligned_base() < self.aligned_end() }
}

/// Memory region classification, mirroring Limine's constants without
/// exposing the Limine dependency to callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryRegionType {
    Usable,
    BootloaderReclaimable,
    AcpiReclaimable,
    AcpiNvs,
    ExecutableAndModules,
    Framebuffer,
    BadMemory,
    Reserved,
}

impl MemoryRegionType {
    /// Pages the buddy may receive immediately (before `release`).
    #[inline]
    pub fn is_immediately_usable(self) -> bool {
        matches!(self, Self::Usable)
    }

    /// Pages the buddy may receive after `LimineData::release` is called.
    #[inline]
    pub fn is_reclaimable(self) -> bool {
        matches!(self, Self::BootloaderReclaimable | Self::AcpiReclaimable)
    }
}

/// Framebuffer geometry and address extracted as plain integers.
#[derive(Debug, Clone, Copy)]
pub struct FramebufferInfo {
    /// Virtual address as given by Limine (inside HHDM).
    pub virt_addr:      u64,
    /// Physical address, derived as `virt_addr - hhdm_offset`.
    pub phys_addr:      u64,
    pub width:          u32,
    pub height:         u32,
    /// Bytes per scanline (may exceed `width * (bpp / 8)`).
    pub pitch:          u32,
    pub bits_per_pixel: u16,
    /// Total byte size of the framebuffer (`height * pitch`).
    pub byte_size:      u64,
    /// Red mask shift, e.g. 11 for RGB565.
    pub r_shift:         u8,
    pub g_shift:         u8,
    pub b_shift:         u8,
}

/// Bootloader name and version copied into fixed-size byte arrays.
/// No C-string pointer survives after harvest.
#[derive(Clone, Copy)]
pub struct BootloaderInfo {
    pub name:        [u8; 64],
    pub name_len:    usize,
    pub version:     [u8; 64],
    pub version_len: usize,
}

impl BootloaderInfo {
    pub fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("<invalid>")
    }
    pub fn version_str(&self) -> &str {
        core::str::from_utf8(&self.version[..self.version_len]).unwrap_or("<invalid>")
    }
}

// 512 entries is generous even for heavily-segmented NUMA systems.
const MAX_REGIONS: usize = 512;

/// All data harvested from Limine responses, stored as owned plain values.
/// No pointer into bootloader memory survives after `harvest()` returns.
pub struct LimineData {
    // ── Always present ────────────────────────────────────────────────────
    pub hhdm_offset:       u64,
    pub kernel_phys_start: u64,
    /// Kernel physical end, rounded up to a page boundary.
    pub kernel_phys_end:   u64,
    pub kernel_virt_start: u64,
    pub kernel_virt_end:   u64,

    // ── Memory map ────────────────────────────────────────────────────────
    regions:      [MemoryRegion; MAX_REGIONS],
    region_count: usize,

    // ── Optional responses ────────────────────────────────────────────────
    pub framebuffer:     Option<FramebufferInfo>,
    /// RSDP physical address. Under base revision 2 Limine returns a virtual
    /// address; we subtract hhdm_offset so callers always get physical.
    pub rsdp_phys:       Option<u64>,
    pub bootloader_info: Option<BootloaderInfo>,

}

impl LimineData {
    // ── Accessors ─────────────────────────────────────────────────────────

    #[inline]
    pub fn regions(&self) -> &[MemoryRegion] {
        &self.regions[..self.region_count]
    }

    pub fn usable_regions(&self) -> impl Iterator<Item = &MemoryRegion> {
        self.regions().iter().filter(|r| r.region_type.is_immediately_usable())
    }

    pub fn reclaimable_regions(&self) -> impl Iterator<Item = &MemoryRegion> {
        self.regions().iter().filter(|r| r.region_type.is_reclaimable())
    }

    // ── harvest ───────────────────────────────────────────────────────────

    /// Extract every value from every Limine response into this struct.
    ///
    /// # Safety
    ///
    /// Must be called while Limine's page mappings remain valid — i.e., before
    /// installing your own page tables. Must be called exactly once; panics on
    /// a second call.
    pub unsafe fn harvest() -> Self {
        assert!(
            !HARVESTED.swap(true, Ordering::SeqCst),
            "LimineData::harvest() called more than once"
        );

        // ── HHDM ──────────────────────────────────────────────────────────
        let hhdm_offset = HHDM_REQUEST
            .response()
            .expect("Limine: no HHDM response")
            .offset;

        // ── Kernel address ────────────────────────────────────────────────
        let kernel_addr = KERNEL_ADDRESS_REQUEST
            .response()
            .expect("Limine: no kernel address response");

        let kernel_phys_start = kernel_addr.physical_base;
        let kernel_virt_start = kernel_addr.virtual_base;

        unsafe extern "C" {
            static __kernel_start: u8;
            static __kernel_end:   u8;
        }
        let raw_virt_start = unsafe { &raw const __kernel_start as u64 };
        let raw_virt_end   = unsafe { &raw const __kernel_end   as u64 };
        let kernel_size    = raw_virt_end.saturating_sub(raw_virt_start);
        let kernel_virt_end = kernel_virt_start + kernel_size;
        // Round end up: partial tail page must not be handed to the buddy.
        let kernel_phys_end = align_up(kernel_phys_start + kernel_size, 4096);

        // ── Memory map ────────────────────────────────────────────────────
        let memmap = MEMMAP_REQUEST
            .response()
            .expect("Limine: no memory map response");

        let mut regions = [MemoryRegion {
            base: 0,
            length: 0,
            region_type: MemoryRegionType::Reserved,
        }; MAX_REGIONS];
        let mut region_count = 0usize;

        for entry in memmap.entries() {
            if region_count >= MAX_REGIONS { break; }
            let region_type = match entry.type_ {
                t if t == MEMMAP_USABLE                  => MemoryRegionType::Usable,
                t if t == MEMMAP_BOOTLOADER_RECLAIMABLE  => MemoryRegionType::BootloaderReclaimable,
                t if t == MEMMAP_ACPI_RECLAIMABLE        => MemoryRegionType::AcpiReclaimable,
                t if t == MEMMAP_ACPI_NVS                => MemoryRegionType::AcpiNvs,
                t if t == MEMMAP_EXECUTABLE_AND_MODULES  => MemoryRegionType::ExecutableAndModules,
                t if t == MEMMAP_FRAMEBUFFER             => MemoryRegionType::Framebuffer,
                t if t == MEMMAP_BAD_MEMORY              => MemoryRegionType::BadMemory,
                // MEMMAP_RESERVED and MEMMAP_MAPPED_RESERVED both fall here.
                _                                        => MemoryRegionType::Reserved,
            };
            regions[region_count] = MemoryRegion {
                base:   entry.base,
                length: entry.length,
                region_type,
            };
            region_count += 1;
        }

        // ── Framebuffer ───────────────────────────────────────────────────
        let framebuffer = FRAMEBUFFER_REQUEST
            .response()
            .and_then(|r| r.framebuffers().first().copied())
            .map(|fb| {
                let virt_addr = fb.address() as u64;
                let phys_addr = virt_addr - hhdm_offset;
                let byte_size = fb.height * fb.pitch;
                FramebufferInfo {
                    virt_addr,
                    phys_addr,
                    width:          fb.width as u32,
                    height:         fb.height as u32,
                    pitch:          fb.pitch as u32,
                    bits_per_pixel: fb.bpp,
                    r_shift:        fb.red_mask_shift,
                    g_shift:        fb.green_mask_shift,
                    b_shift:        fb.blue_mask_shift,
                    byte_size,
                }
            });

        // ── RSDP ──────────────────────────────────────────────────────────
        // Under base revision 2 the address is virtual; subtract hhdm_offset
        // so callers always receive a consistent physical address.
        let rsdp_phys = RSDP_REQUEST
            .response()
            .map(|r| (r.address as usize as u64).saturating_sub(hhdm_offset));

        // ── Bootloader info ───────────────────────────────────────────────
        let bootloader_info = BOOTLOADER_INFO_REQUEST
            .response()
            .map(|r| {
                let mut info = BootloaderInfo {
                    name:        [0u8; 64],
                    name_len:    0,
                    version:     [0u8; 64],
                    version_len: 0,
                };
                let name_bytes    = r.name().as_bytes();
                let name_len      = name_bytes.len().min(63);
                info.name[..name_len].copy_from_slice(&name_bytes[..name_len]);
                info.name_len     = name_len;

                let version_bytes = r.version().as_bytes();
                let version_len   = version_bytes.len().min(63);
                info.version[..version_len].copy_from_slice(&version_bytes[..version_len]);
                info.version_len  = version_len;

                info
            });

        LimineData {
            hhdm_offset,
            kernel_phys_start,
            kernel_phys_end,
            kernel_virt_start,
            kernel_virt_end,
            regions,
            region_count,
            framebuffer,
            rsdp_phys,
            bootloader_info,
        }
    }

    // ── release ───────────────────────────────────────────────────────────

    /// Feed all bootloader-reclaimable and ACPI-reclaimable pages into the
    /// buddy allocator, then consume `self`.
    ///
    /// Call this after:
    /// - The heap (buddy + TLSF) is fully initialized.
    /// - The VMM is up and no longer depends on Limine's page tables.
    /// - All fields you still need have been copied out of `self`.
    ///
    /// After this returns, the physical pages that held Limine's own structures
    /// (page tables, responses, boot stack) are in the free pool.
    ///
    /// # Safety
    ///
    /// No pointer into bootloader-reclaimable physical memory may be live
    /// anywhere in the kernel at the time of this call. Guaranteed by the
    /// design of this module — no Limine pointer escapes `harvest`.
    pub unsafe fn release(self) {
        let hhdm_offset = self.hhdm_offset;

        // Read the current stack pointer. Any reclaimable page at or below
        // this address is live stack memory and must not be freed.
        let current_sp: u64;
        unsafe {
            core::arch::asm!("mov {}, rsp", out(reg) current_sp, options(nostack, nomem));
        }
        // Convert SP to physical; round down to page boundary.
        let sp_phys_page = (current_sp - hhdm_offset) & !0xFFF;

        let mut buddy = BUDDY.lock();

        for region in self.reclaimable_regions() {
            let base = region.aligned_base();
            let end  = region.aligned_end();
            if base >= end { continue; }

            // Skip the sub-1MiB region. It precedes the buddy's base address
            // (seeded from Usable regions starting at 0x53000) and contains
            // real-mode IVT, BDA, EBDA, and ROM shadow — none of which are
            // safe general-purpose heap memory on x86.
            if base < 0x100000 {
                continue;
            }

            // Skip any region containing the current stack pointer (see prior fix).
            if sp_phys_page >= base && sp_phys_page < end {
                continue;
            }

            let page_count = ((end - base) / 4096) as usize;
            if page_count == 0 { continue; }

            unsafe {
                buddy.add_region((hhdm_offset + base) as usize, page_count);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usable(base: u64, length: u64) -> MemoryRegion {
        MemoryRegion { base, length, region_type: MemoryRegionType::Usable }
    }

    // ── MemoryRegion ──────────────────────────────────────────────────────────

    #[test_case]
    fn end_is_base_plus_length() {
        assert_eq!(usable(0x1000, 0x3000).end(), 0x4000);
        assert_eq!(usable(0, 0).end(), 0);
    }

    #[test_case]
    fn aligned_base_rounds_up_to_page_boundary() {
        assert_eq!(usable(0x1001, 0x5000).aligned_base(), 0x2000);
        assert_eq!(usable(0x0FFF, 0x5000).aligned_base(), 0x1000);
    }

    #[test_case]
    fn aligned_base_of_page_aligned_region_is_unchanged() {
        assert_eq!(usable(0x2000, 0x1000).aligned_base(), 0x2000);
        assert_eq!(usable(0,      0x1000).aligned_base(), 0);
    }

    #[test_case]
    fn aligned_end_rounds_down_to_page_boundary() {
        // base=0x1000, length=0x1FFF -> end=0x2FFF, aligned_end=0x2000
        assert_eq!(usable(0x1000, 0x1FFF).aligned_end(), 0x2000);
    }

    #[test_case]
    fn aligned_end_of_page_aligned_region_is_unchanged() {
        assert_eq!(usable(0x1000, 0x2000).aligned_end(), 0x3000);
    }

    #[test_case]
    fn has_pages_true_when_at_least_one_full_page_fits() {
        assert!(usable(0x1000, 0x1000).has_pages()); // exactly one page
        assert!(usable(0x1000, 0x2000).has_pages()); // two pages
    }

    #[test_case]
    fn has_pages_false_when_region_has_no_full_page() {
        // aligned_base=0x2000, end=0x2000 -> no page
        assert!(!usable(0x1001, 0x0FFF).has_pages());
        assert!(!usable(0x1000, 0).has_pages());
    }

    // ── MemoryRegionType ──────────────────────────────────────────────────────

    #[test_case]
    fn only_usable_is_immediately_usable() {
        assert!( MemoryRegionType::Usable.is_immediately_usable());
        assert!(!MemoryRegionType::BootloaderReclaimable.is_immediately_usable());
        assert!(!MemoryRegionType::AcpiReclaimable.is_immediately_usable());
        assert!(!MemoryRegionType::Reserved.is_immediately_usable());
        assert!(!MemoryRegionType::Framebuffer.is_immediately_usable());
    }

    #[test_case]
    fn reclaimable_types_are_reclaimable() {
        assert!( MemoryRegionType::BootloaderReclaimable.is_reclaimable());
        assert!( MemoryRegionType::AcpiReclaimable.is_reclaimable());
        assert!(!MemoryRegionType::Usable.is_reclaimable());
        assert!(!MemoryRegionType::Reserved.is_reclaimable());
        assert!(!MemoryRegionType::ExecutableAndModules.is_reclaimable());
    }
}
