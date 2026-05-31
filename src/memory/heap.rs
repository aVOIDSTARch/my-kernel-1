// v0.0.4
use abalone::{buddy::BUDDY, tlsf::TlsfAllocator};
use bitwise::align::{align_down, align_up};

use crate::{limine_data::{MemoryRegion, MemoryRegionType}, serial_println};

#[global_allocator]
static HEAP: TlsfAllocator = TlsfAllocator::new();

/// Initialise the kernel heap.
///
/// Feeds all immediately-usable physical memory regions (via HHDM) into the
/// buddy allocator, punching out the kernel's own pages, then carves a 4 MiB
/// pool from the buddy to bootstrap the TLSF heap.
///
/// Bootloader-reclaimable regions are intentionally excluded here; they are
/// added by `LimineData::release()` after the VMM is up and all Limine
/// response pointers have been discarded.
///
/// After this returns, `Box`, `Vec`, `String`, etc. are available.
pub fn init(
    regions:           &[MemoryRegion],
    kernel_phys_start: u64,
    kernel_phys_end:   u64,
    hhdm_offset:       u64,
) {
    // Defensive alignment — limine_data already guarantees page alignment on
    // usable entries, but correctness should not depend on call-site discipline.
    let kernel_phys_start = align_down(kernel_phys_start, 4096);
    let kernel_phys_end   = align_up(kernel_phys_end, 4096);

    {
        let mut buddy = BUDDY.lock();

        for region in regions {
            if region.region_type != MemoryRegionType::Usable
                // Skip usable regions that are entirely consumed by the kernel.
                {
                    serial_println!("[memmap] skipping {:#x}+{:#x} {:?}",
                        region.base, region.length, region.region_type);
                    continue;
                }
            else {
                serial_println!("[memmap] using {:#x}+{:#x} {:?}",
                    region.base, region.length, region.region_type);
            }

            let base = region.aligned_base();
            let end  = region.aligned_end();
            if base >= end { continue; }

            // Add in up to two parts, punching out the kernel range.
            //
            // Case 1: region extends below the kernel.
            if base < kernel_phys_start {
                let part_end   = end.min(kernel_phys_start);
                let page_count = ((part_end - base) / 4096) as usize;
                if page_count > 0 {
                    unsafe {
                        buddy.add_region((hhdm_offset + base) as usize, page_count);
                    }
                }
            }
            // Case 2: region extends above the kernel.
            if end > kernel_phys_end {
                let part_start = base.max(kernel_phys_end);
                let page_count = ((end - part_start) / 4096) as usize;
                if page_count > 0 {
                    unsafe {
                        buddy.add_region((hhdm_offset + part_start) as usize, page_count);
                    }
                }
            }
        }
    }

    // Carve 4 MiB (2^10 pages) from the buddy to seed the TLSF sub-page heap.
    unsafe { HEAP.init(10); }
}
