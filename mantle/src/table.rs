// v0.0.4
// Memory ordering: x86_64's TSO model guarantees that stores to page-table
// memory are observed by the MMU walker in program order on the local CPU.
// read_volatile / write_volatile are sufficient to prevent compiler reordering.
// A port to weakly-ordered architectures (AArch64, RISC-V) requires DSB/FENCE
// instructions before TLB invalidation.
use core::ptr;

/// A single level of the x86_64 4-level page table hierarchy.
///
/// Must be 4 KiB aligned and exactly 4 KiB in size so the buddy allocator's
/// single-page allocation (`order = 0`) can back it directly.
#[repr(C, align(4096))]
pub struct PageTable {
    entries: [u64; 512],
}

impl PageTable {
    /// Read one entry. Volatile to prevent the compiler from caching the value
    /// across TLB-modifying operations.
    #[inline]
    pub fn read(&self, index: usize) -> u64 {
        unsafe { ptr::read_volatile(&self.entries[index]) }
    }

    /// Write one entry. Volatile to prevent the compiler from eliding the store.
    #[inline]
    pub fn write(&mut self, index: usize, value: u64) {
        unsafe { ptr::write_volatile(&mut self.entries[index], value) }
    }

    /// Zero all 512 entries using a single memset-equivalent store sequence.
    #[inline]
    pub fn zero(&mut self) {
        // Safety: entries is [u64; 512]; all-zero bits are valid for u64.
        // This frame is freshly allocated and not yet visible to the MMU,
        // so non-volatile memset semantics are correct here.
        unsafe {
            ptr::write_bytes(self.entries.as_mut_ptr(), 0, 512);
        }
    }
}
