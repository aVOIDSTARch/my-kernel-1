// v0.0.4
use bitwise::paging::pte_flags;

/// Page protection flags for a kernel mapping.
///
/// Wraps raw PTE flag bits. `PRESENT` is always added by the walker at map time;
/// callers only supply the access/cache policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Protection(pub u64);

impl Protection {
    /// Read-only kernel data. No-execute, not writable. Global (not flushed on CR3 reload).
    pub const KERNEL_RO: Self = Self(pte_flags::NO_EXECUTE | pte_flags::GLOBAL);

    /// Read-write kernel data. No-execute, writable. Global.
    pub const KERNEL_RW: Self = Self(pte_flags::WRITABLE | pte_flags::NO_EXECUTE | pte_flags::GLOBAL);

    /// Read-execute kernel code. Executable (NX clear), not writable. Global.
    pub const KERNEL_RX: Self = Self(pte_flags::GLOBAL);

    /// Read-write-execute. Use only during early boot; remove once code is loaded.
    #[deprecated = "W^X violation. Use KERNEL_RX for code or KERNEL_RW for data. \
                    Only acceptable during early boot JIT stubs with a known lifetime."]
    pub const KERNEL_RWX: Self = Self(pte_flags::WRITABLE);

    /// Strict uncacheable MMIO. Writable, no-execute, PWT+PCD set (PAT index 3 = UC strong).
    /// Use for device register BARs that require strongly ordered stores.
    pub const MMIO_UC: Self = Self(
        pte_flags::WRITABLE
            | pte_flags::NO_EXECUTE
            | pte_flags::CACHE_DISABLE
            | pte_flags::WRITE_THROUGH,
    );

    /// Write-combining MMIO. Writable, no-execute, PWT+PCD+PAT set (PAT index 7 = WC).
    /// Requires `init_pat()` to have been called. Use for framebuffers — coalesces
    /// pixel stores into burst transactions for 4-8x higher throughput than UC.
    pub const MMIO_WC: Self = Self(
        pte_flags::WRITABLE
            | pte_flags::NO_EXECUTE
            | pte_flags::CACHE_DISABLE
            | pte_flags::WRITE_THROUGH
            | pte_flags::PAT,
    );

    /// Raw flag bits to OR into a leaf PTE (combined with PRESENT by the walker).
    #[inline]
    pub fn bits(self) -> u64 {
        self.0
    }
}
