//! Process struct and table trait implementations.
//!
//! `Process` is the address-space owner. It implements all four traits
//! required by `seastar::table::ProcessTable`:
//!
//! - `HasGeneration`   — read the slot-reuse generation counter.
//! - `HasPid`          — read the process's PID.
//! - `StampGeneration` — write path called only by `ProcessTable::insert`.
//! - `StampPid`        — write path called only by `ProcessTable::insert`.
//!
//! The generation field is an `AtomicU64` so `ProcessRef::get` can read it
//! without holding any lock. The PID field is a plain `ProcessId` (not atomic)
//! because it is written exactly once during insert while the slot is
//! exclusively owned, and is never mutated thereafter.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::{
    flags::ProcessFlags,
    ids::{ProcessId, ThreadId},
    state::ProcessState,
    table::{HasGeneration, HasPid, StampGeneration, StampPid},
    thread::Thread,
};

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// An address space owner. Contains one or more threads.
///
/// ## Field access discipline
///
/// - `id`         — written once by `StampPid`; read-only thereafter.
/// - `generation` — written once by `StampGeneration`; read atomically
///                  by `ProcessRef::get` without any lock.
/// - `state`      — mutable; protected by `pincer::psync::IrqMutex`.
/// - `flags`      — mutable; protected by `pincer::psync::IrqMutex`.
/// - `threads`    — mutable; protected by `pincer::psync::IrqMutex`.
///
/// ## CR3
///
/// `cr3 = 0` means "kernel process; never switch CR3". The idle process
/// uses this. User processes receive a non-zero physical page table root
/// from the VMM.
pub struct Process {
    /// The process's PID. Set once by `ProcessTable::insert` via `StampPid`.
    /// Not atomic — written once while exclusively owned, read-only after.
    pub id: ProcessId,

    /// Slot-reuse generation counter. Set once by `ProcessTable::insert` via
    /// `StampGeneration`. Read atomically by `ProcessRef::get`.
    pub generation: AtomicU64,

    /// Physical address of the PML4 page table root.
    /// 0 for kernel processes (no address space switch).
    pub cr3: u64,

    pub state:  pincer::psync::SpinMutex<ProcessState>,
    pub flags:  pincer::psync::SpinMutex<ProcessFlags>,

    #[cfg(feature = "alloc")]
    pub threads: pincer::psync::SpinMutex<Vec<Thread>>,

    pub exit_code: core::sync::atomic::AtomicI32,
}

// ── Table trait implementations ───────────────────────────────────────────────

impl HasGeneration for Process {
    #[inline]
    fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
}

impl HasPid for Process {
    #[inline]
    fn pid(&self) -> u64 {
        self.id.as_u64()
    }
}

impl StampGeneration for Process {
    /// # Safety
    /// Must only be called by `ProcessTable::insert` on a freshly allocated,
    /// exclusively owned instance before the slot is published in the index.
    unsafe fn stamp_generation(this: &mut Self, gen: u64) {
        // Release ordering: the generation write is visible to any thread that
        // subsequently reads it with Acquire (as ProcessRef::get does).
        this.generation.store(gen, Ordering::Release);
    }
}

impl StampPid for Process {
    /// # Safety
    /// Must only be called by `ProcessTable::insert` on a freshly allocated,
    /// exclusively owned instance, after `stamp_generation`.
    unsafe fn stamp_pid(this: &mut Self, pid: u64) {
        // Plain write: the slot is exclusively owned at this point. No
        // concurrent reader exists until the slot is published in the index,
        // which happens after this call returns.
        this.id = ProcessId::from_raw(pid);
    }
}

// ── Constructor ───────────────────────────────────────────────────────────────

#[cfg(feature = "alloc")]
impl Process {
    /// Construct a kernel process ready for insertion into `ProcessTable`.
    ///
    /// `id` and `generation` are left as `INVALID`/`0` here — they will be
    /// overwritten by `ProcessTable::insert` via the stamp traits. The caller
    /// must not read `self.id` or `self.generation` before insert returns.
    ///
    /// `cr3 = 0` indicates a kernel process; the context switcher will not
    /// perform a CR3 swap for this process.
    pub fn new_kernel() -> Self {
        Self {
            id:         ProcessId::INVALID, // stamped by insert
            generation: AtomicU64::new(0),  // stamped by insert
            cr3:        0,
            state:      pincer::psync::SpinMutex::new(ProcessState::Created),
            flags:      pincer::psync::SpinMutex::new(ProcessFlags::KERNEL_PROCESS),
            threads:    pincer::psync::SpinMutex::new(Vec::new()),
            exit_code:  core::sync::atomic::AtomicI32::new(0),
        }
    }

    /// Construct a user process with a given CR3.
    /// `id` and `generation` are stamped by `ProcessTable::insert`.
    pub fn new_user(cr3: u64) -> Self {
        Self {
            id:         ProcessId::INVALID,
            generation: AtomicU64::new(0),
            cr3,
            state:      pincer::psync::SpinMutex::new(ProcessState::Created),
            flags:      pincer::psync::SpinMutex::new(ProcessFlags::empty()),
            threads:    pincer::psync::SpinMutex::new(Vec::new()),
            exit_code:  core::sync::atomic::AtomicI32::new(0),
        }
    }

    /// Returns `Some(cr3)` if this process needs a CR3 swap on context switch.
    /// Returns `None` for kernel processes.
    pub fn cr3_for_switch(&self) -> Option<u64> {
        if self.cr3 == 0 { None } else { Some(self.cr3) }
    }
}

impl core::fmt::Debug for Process {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Process")
            .field("id",         &self.id)
            .field("generation", &self.generation.load(Ordering::Relaxed))
            .field("cr3",        &format_args!("{:#018x}", self.cr3))
            .field("state",      &*self.state.lock())
            .finish()
    }
}
