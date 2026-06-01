//! Identity newtypes for processes and threads.
//!
//! PID issuance is now owned by `ProcessTable::insert`. `ProcessId::new()`
//! has been removed to prevent callers from allocating PIDs outside the table,
//! which would break the uniqueness guarantee. Use `ProcessId::from_raw` only
//! inside trait implementations that respond to the table's stamp calls.

use core::sync::atomic::{AtomicU64, Ordering};

// ThreadId still self-allocates — threads are not yet table-managed.
static NEXT_TID: AtomicU64 = AtomicU64::new(1);

// ── ProcessId ─────────────────────────────────────────────────────────────────

/// Unique identifier for a process (address space owner).
///
/// PID 0 is `INVALID` and is never issued by `ProcessTable`.
/// Width is `u64` to match `seastar::table::Pid`. Truncate to `u32`
/// only at the syscall ABI boundary via `Pid::as_u32_for_syscall`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct ProcessId(u64);

impl ProcessId {
    /// The invalid/unset sentinel. Never issued to a live process.
    pub const INVALID: Self = Self(0);

    /// Construct from a raw value. Called only by `StampPid` implementations
    /// in response to `ProcessTable::insert`. Do not call at other sites.
    #[inline]
    pub fn from_raw(v: u64) -> Self {
        debug_assert!(v != 0, "ProcessId::from_raw: 0 is reserved for INVALID");
        Self(v)
    }

    #[inline]
    pub fn as_u64(self) -> u64 { self.0 }

    /// Truncate to `u32` for POSIX syscall returns.
    /// Acceptable at the ABI boundary; never use inside the kernel.
    #[inline]
    pub fn as_u32_for_syscall(self) -> u32 { self.0 as u32 }
}

impl core::fmt::Display for ProcessId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "PID({})", self.0)
    }
}

// ── ThreadId ──────────────────────────────────────────────────────────────────

/// Unique identifier for a thread (schedulable unit within a process).
///
/// TID 0 is `IDLE` and is never allocated by `ThreadId::new()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct ThreadId(u64);

impl ThreadId {
    pub const IDLE: Self = Self(0);

    /// Allocate the next available TID. Monotonically increasing; never reused.
    pub fn new() -> Self {
        Self(NEXT_TID.fetch_add(1, Ordering::Relaxed))
    }

    #[inline]
    pub fn as_u64(self) -> u64 { self.0 }
}

impl core::fmt::Display for ThreadId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "TID({})", self.0)
    }
}
