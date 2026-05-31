// v0.0.2
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

    /// Zero all 512 entries.
    #[inline]
    pub fn zero(&mut self) {
        for i in 0..512 {
            self.write(i, 0);
        }
    }
}
