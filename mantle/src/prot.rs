use bitwise::paging::pte_flags;

/// Page protection flags for a kernel mapping.
///
/// Wraps raw PTE flag bits. `PRESENT` is always added by the walker at map time;
/// callers only supply the access/cache policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Protection(pub u64);

impl Protection {
    /// Read-only kernel data. No-execute, not writable.
    pub const KERNEL_RO: Self = Self(pte_flags::NO_EXECUTE);

    /// Read-write kernel data. No-execute, writable.
    pub const KERNEL_RW: Self = Self(pte_flags::WRITABLE | pte_flags::NO_EXECUTE);

    /// Read-execute kernel code. Executable (NX clear), not writable.
    pub const KERNEL_RX: Self = Self(0);

    /// Read-write-execute. Use only during early boot; remove once code is loaded.
    pub const KERNEL_RWX: Self = Self(pte_flags::WRITABLE);

    /// Memory-mapped I/O region. Writable, no-execute, cache disabled, write-through.
    pub const MMIO: Self = Self(
        pte_flags::WRITABLE
            | pte_flags::NO_EXECUTE
            | pte_flags::CACHE_DISABLE
            | pte_flags::WRITE_THROUGH,
    );

    /// Raw flag bits to OR into a leaf PTE (combined with PRESENT by the walker).
    #[inline]
    pub fn bits(self) -> u64 {
        self.0
    }
}
