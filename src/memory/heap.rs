use abalone::{buddy::BUDDY, tlsf::TlsfAllocator};
use bitwise::align::{align_down, align_up};
use limine::memmap::{Entry, MEMMAP_BOOTLOADER_RECLAIMABLE, MEMMAP_USABLE};

#[global_allocator]
static HEAP: TlsfAllocator = TlsfAllocator::new();

/// Initialise the kernel heap.
///
/// Feeds all usable and bootloader-reclaimable physical memory regions (via
/// HHDM) into the buddy allocator, skipping the kernel's own pages, then
/// carves a 4 MiB pool from the buddy to bootstrap the TLSF heap.
///
/// After this returns, `Box`, `Vec`, `String`, etc. are available.
pub fn init(
    entries: &[&Entry],
    kernel_phys_start: u64,
    kernel_phys_end: u64,
    hhdm_offset: u64,
) {
    {
        let mut buddy = BUDDY.lock();

        for entry in entries {
            if entry.type_ != MEMMAP_USABLE && entry.type_ != MEMMAP_BOOTLOADER_RECLAIMABLE {
                continue;
            }

            let base = align_up(entry.base, 4096);
            let end  = align_down(entry.base + entry.length, 4096);
            if base >= end { continue; }

            // Add the region in up to two parts, punching out the kernel range.
            if base < kernel_phys_start {
                let part_end   = end.min(kernel_phys_start);
                let page_count = ((part_end - base) / 4096) as usize;
                if page_count > 0 {
                    unsafe {
                        buddy.add_region((hhdm_offset + base) as usize, page_count);
                    }
                }
            }
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
