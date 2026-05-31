// v0.0.2
use limine::memmap::{Entry, MEMMAP_BOOTLOADER_RECLAIMABLE, MEMMAP_USABLE};
use spin::Mutex;

// 64 GiB / 4 KiB pages = 16,777,216 pages = 2,097,152 u8 bytes = 2 MiB bitmap
const MAX_PAGES: usize = 64 * 1024 * 1024 * 1024 / 4096;
const BITMAP_SIZE: usize = MAX_PAGES / 8;

static BITMAP: Mutex<[u8; BITMAP_SIZE]> = Mutex::new([0xFF; BITMAP_SIZE]); // All reserved initially
static mut HHDM_OFFSET: u64 = 0;
static mut TOTAL_FREE_PAGES: u64 = 0;

fn page_index(phys_addr: u64) -> usize {
    (phys_addr / 4096) as usize
}

fn mark_free(bitmap: &mut [u8; BITMAP_SIZE], page: usize) {
    bitmap[page / 8] &= !(1 << (page % 8));
}

fn mark_used(bitmap: &mut [u8; BITMAP_SIZE], page: usize) {
    bitmap[page / 8] |= 1 << (page % 8);
}

fn is_free(bitmap: &[u8; BITMAP_SIZE], page: usize) -> bool {
    (bitmap[page / 8] & (1 << (page % 8))) == 0
}

fn align_up(addr: u64, align: u64) -> u64 {
    (addr + align - 1) & !(align - 1)
}

fn align_down(addr: u64, align: u64) -> u64 {
    addr & !(align - 1)
}

pub fn init(
    entries: &[&Entry],
    kernel_phys_start: u64,
    kernel_phys_end: u64,
    hhdm_offset: u64,
) {
    unsafe { HHDM_OFFSET = hhdm_offset; }

    let mut bitmap = BITMAP.lock();
    let mut free_pages: u64 = 0;

    for entry in entries {
        if entry.type_ != MEMMAP_USABLE {
            continue;
        }

        let base = align_up(entry.base, 4096);
        let end  = align_down(entry.base + entry.length, 4096);

        if base >= end { continue; }

        let start_page = page_index(base);
        let end_page   = page_index(end);

        if end_page > MAX_PAGES { continue; }

        for page in start_page..end_page {
            let phys = page as u64 * 4096;
            // Exclude the kernel image from the free pool.
            if phys >= kernel_phys_start && phys < kernel_phys_end {
                continue;
            }
            mark_free(&mut bitmap, page);
            free_pages += 1;
        }
    }

    unsafe { TOTAL_FREE_PAGES = free_pages; }
}

/// Reclaim pages previously marked BOOTLOADER_RECLAIMABLE.
/// Only call after all Limine response data has been consumed.
pub fn reclaim_bootloader_memory(entries: &[&Entry]) {
    let mut bitmap = BITMAP.lock();
    let mut reclaimed: u64 = 0;

    for entry in entries {
        if entry.type_ != MEMMAP_BOOTLOADER_RECLAIMABLE {
            continue;
        }

        let base = align_up(entry.base, 4096);
        let end  = align_down(entry.base + entry.length, 4096);

        if base >= end { continue; }

        let start_page = page_index(base);
        let end_page   = page_index(end);

        if end_page > MAX_PAGES { continue; }

        for page in start_page..end_page {
            mark_free(&mut bitmap, page);
            reclaimed += 1;
        }
    }

    unsafe { TOTAL_FREE_PAGES += reclaimed; }
}

/// Allocate a single 4 KiB physical page.
/// Returns the physical address of the allocated page, or None if OOM.
pub fn alloc_page() -> Option<u64> {
    let mut bitmap = BITMAP.lock();
    for i in 0..MAX_PAGES {
        if is_free(&bitmap, i) {
            mark_used(&mut bitmap, i);
            unsafe {
                if TOTAL_FREE_PAGES > 0 { TOTAL_FREE_PAGES -= 1; }
            }
            return Some(i as u64 * 4096);
        }
    }
    None
}

/// Free a single 4 KiB physical page.
/// Safety: phys_addr must have been returned by alloc_page and not yet freed.
pub unsafe fn free_page(phys_addr: u64) {
    let page = page_index(phys_addr);
    let mut bitmap = BITMAP.lock();
    mark_free(&mut bitmap, page);
    unsafe { TOTAL_FREE_PAGES += 1; }
}

/// Convert a physical address to a virtual address via HHDM.
pub fn phys_to_virt(phys: u64) -> u64 {
    unsafe { phys + HHDM_OFFSET }
}

pub fn free_pages() -> u64 {
    unsafe { TOTAL_FREE_PAGES }
}
