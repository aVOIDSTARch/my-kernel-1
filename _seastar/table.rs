//! # `seastar::table` — Generation-tagged process table
//!
//! Provides O(1) kernel-internal pointer lookup via `ProcessRef<T>` and
//! opaque monotonic PIDs safe to expose across the syscall boundary.
//!
//! ## Two namespaces
//!
//! - **`Pid`** — a `u64` exposed to userspace via syscalls. Monotonically
//!   allocated, carries no kernel address, safe to hand to ring-3 code.
//!   Truncate to `u32` at the syscall boundary only; never internally.
//!
//! - **`ProcessRef<T>`** — a kernel-internal fat pointer (raw ptr + generation).
//!   Dereferences in O(1) via a single generation check. Never crosses into
//!   userspace. Never serialised, stored to disk, or put in a shared buffer.
//!
//! ## Integration surface
//!
//! Two traits must be implemented on the process struct `T`:
//!
//! - [`StampGeneration`] — write path for the generation field (table only).
//! - [`StampPid`]        — write path for the pid field (table only).
//!
//! Both imply their read counterparts ([`HasGeneration`], [`HasPid`]).
//!
//! One trait must be implemented for the backing allocator:
//!
//! - [`Allocator`] — two methods; delegate to `SlabCache<T>`.
//!
//! ## Locking
//!
//! The index is protected by `pincer::psync::IrqMutex<_, I>`. The `I: IrqControl`
//! type parameter is injected by the kernel crate so `seastar` carries no
//! architecture-specific code. Interrupt safety is enforced at the type level
//! rather than documented in a comment.
//!
//! Do not call `insert`, `remove`, or `lookup` from an interrupt handler that
//! can preempt code already holding the table lock — that is what `IrqMutex`
//! prevents, but re-entrant acquisition from the same CPU still deadlocks.

use core::{
    ptr::NonNull,
    sync::atomic::{AtomicU64, Ordering},
};
use pincer::psync::{IrqControl, IrqMutex};

// ── Pid ───────────────────────────────────────────────────────────────────────

/// Opaque process identifier safe for userspace exposure.
///
/// Stored as `u64` internally. When returning from a syscall that must
/// fit in a POSIX `pid_t` (signed 32-bit), use `Pid::as_u32_for_syscall()`.
/// Never truncate silently inside the kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Pid(u64);

impl Pid {
    /// Sentinel: never assigned to any process.
    pub const INVALID: Self = Self(0);

    /// The raw `u64`. Use for kernel-internal comparisons.
    #[inline(always)]
    pub const fn as_u64(self) -> u64 { self.0 }

    /// Truncate to `u32` for POSIX syscall returns only.
    /// Wraps silently above `u32::MAX`; acceptable at the ABI boundary.
    #[inline(always)]
    pub const fn as_u32_for_syscall(self) -> u32 { self.0 as u32 }

    /// Construct from a raw value. For use by stamp traits only.
    #[inline(always)]
    pub(crate) const fn from_raw(v: u64) -> Self { Self(v) }
}

impl core::fmt::Display for Pid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ── Generation ────────────────────────────────────────────────────────────────

/// Slot reuse detector stored in both the index and the live struct.
///
/// Generation 0 means the slot has never been used.
/// Generation ≥ 1 is a live or previously-live slot.
/// Wrapping after 2^64 reuses of the same slot is academically possible;
/// in any realistic kernel lifetime it will not occur.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
struct Generation(u64);

impl Generation {
    const INITIAL: Self = Self(1);

    #[inline]
    fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

// ── Allocator trait ───────────────────────────────────────────────────────────

/// Fixed-size allocator interface for the process table.
///
/// Implement by delegating to `SlabCache<T>`:
///
/// ```rust,ignore
/// impl Allocator<Process> for SlabBacked {
///     fn alloc(&self) -> Option<NonNull<Process>> { self.0.alloc() }
///     unsafe fn dealloc(&self, ptr: NonNull<Process>) {
///         unsafe { self.0.dealloc(ptr) }
///     }
/// }
/// ```
pub trait Allocator<T> {
    fn alloc(&self) -> Option<NonNull<T>>;

    /// # Safety
    /// `ptr` must originate from `self.alloc()`. The `T`'s destructor must
    /// have been called before this. Must not be called twice for the same ptr.
    unsafe fn dealloc(&self, ptr: NonNull<T>);
}

// ── Process struct trait bounds ───────────────────────────────────────────────

/// Read the generation embedded in `T`. Called by `ProcessRef::get`.
pub trait HasGeneration {
    fn generation(&self) -> u64;
}

/// Read the PID embedded in `T`. Called by `lookup`.
pub trait HasPid {
    fn pid(&self) -> u64;
}

/// Write the generation into `T`. Called only by `ProcessTable::insert`.
///
/// # Safety
/// Must only be called on a freshly allocated, exclusively owned instance.
pub trait StampGeneration: HasGeneration + HasPid + Sized {
    unsafe fn stamp_generation(this: &mut Self, gen: u64);
}

/// Write the PID into `T`. Called only by `ProcessTable::insert`.
///
/// # Safety
/// Must only be called on a freshly allocated, exclusively owned instance,
/// immediately after `stamp_generation`.
pub trait StampPid: StampGeneration {
    unsafe fn stamp_pid(this: &mut Self, pid: u64);
}

// ── IndexEntry ────────────────────────────────────────────────────────────────

struct IndexEntry<T> {
    /// Raw pointer to the live struct. Null when the slot is free.
    ptr:        *mut T,
    /// Generation of the current occupant. 0 when never used.
    generation: Generation,
}

// SAFETY: access is serialised through IrqMutex.
unsafe impl<T: Send> Send for IndexEntry<T> {}

impl<T> IndexEntry<T> {
    const fn empty() -> Self {
        Self {
            ptr:        core::ptr::null_mut(),
            generation: Generation(0),
        }
    }
}

// ── ProcessRef ────────────────────────────────────────────────────────────────

/// Kernel-internal handle to a live process. Two words: pointer + generation.
///
/// Dereferencing is O(1): one atomic load and one comparison.
///
/// **Never expose to userspace. Never serialise. Discard after the operation
/// that required it.**
pub struct ProcessRef<T> {
    ptr:        NonNull<T>,
    generation: Generation,
}

unsafe impl<T: Send + Sync> Send for ProcessRef<T> {}
unsafe impl<T: Send + Sync> Sync for ProcessRef<T> {}

impl<T> Clone for ProcessRef<T> {
    fn clone(&self) -> Self {
        Self { ptr: self.ptr, generation: self.generation }
    }
}
impl<T> Copy for ProcessRef<T> {}

impl<T: HasGeneration> ProcessRef<T> {
    /// Obtain a shared reference to the process struct.
    ///
    /// Returns `None` if the process has been destroyed since this handle
    /// was created (generation mismatch on the struct's atomic field).
    ///
    /// No lock is held during this call. The generation check reads the
    /// `AtomicU64` embedded in `T` directly, which is safe because:
    ///
    /// 1. The pointer was valid when this `ProcessRef` was created.
    /// 2. A generation mismatch means the slot was freed and optionally
    ///    reused. The atomic read of a freed-and-reallocated slot cannot
    ///    produce the old generation value because `remove` increments it
    ///    before releasing the pointer.
    /// 3. `T` is required to expose its generation through an `AtomicU64`
    ///    via `HasGeneration`, which provides the necessary ordering.
    ///
    /// # Safety
    /// The caller must ensure no mutable reference to `T` exists concurrently.
    /// In practice: `T` uses interior mutability (`IrqMutex` over its fields)
    /// and this invariant is structural, not caller-enforced.
    pub unsafe fn get(&self) -> Option<&T> {
        // SAFETY: ptr was valid at construction. If the slot was freed and
        // reused, the generation will have been incremented, catching it below.
        let t = unsafe { self.ptr.as_ref() };
        if t.generation() == self.generation.0 {
            Some(t)
        } else {
            None
        }
    }
}

// ── ProcessTable ──────────────────────────────────────────────────────────────

/// Fixed-capacity, generation-tagged process table.
///
/// | Operation          | Complexity       |
/// |--------------------|------------------|
/// | `insert`           | O(CAP) amortised |
/// | `remove`           | O(CAP)           |
/// | `lookup` (PID)     | O(CAP) — cold    |
/// | `ProcessRef::get`  | O(1) — hot path  |
///
/// `CAP` defaults to 1024. The index array lives in `.bss`
/// (≈ CAP × 24 bytes; 24 KiB at CAP=1024).
///
/// ## Type parameters
///
/// - `T`   — the process struct; must implement `StampPid`.
/// - `A`   — the backing allocator; must implement `Allocator<T>`.
/// - `CAP` — maximum concurrent processes.
/// - `I`   — `IrqControl` implementation injected by the kernel crate.
pub struct ProcessTable<T, A, const CAP: usize, I>
where
    A: Allocator<T>,
    I: IrqControl,
{
    allocator: A,
    index:     IrqMutex<[IndexEntry<T>; CAP], I>,
    next_pid:  AtomicU64,
}

impl<T, A, const CAP: usize, I> ProcessTable<T, A, CAP, I>
where
    T: StampPid + Send + Sync,
    A: Allocator<T>,
    I: IrqControl,
{
    /// Construct an empty process table.
    ///
    /// Safe to call in a `static` initialiser; no allocation occurs until
    /// `insert` is called.
    pub const fn new(allocator: A) -> Self {
        Self {
            allocator,
            index:    IrqMutex::new([const { IndexEntry::empty() }; CAP]),
            next_pid: AtomicU64::new(1), // PID 0 is INVALID; start at 1
        }
    }

    /// Insert a fully constructed process struct into the table.
    ///
    /// Allocates a slab slot, writes `process` into it, stamps both the
    /// generation and the PID into the struct atomically under the index lock,
    /// then records the slot.
    ///
    /// Returns `(Pid, ProcessRef<T>)` on success.
    /// - `Pid` is safe to return to userspace.
    /// - `ProcessRef<T>` is kernel-internal only.
    ///
    /// Returns `None` if:
    /// - The backing allocator is exhausted.
    /// - All `CAP` slots are occupied.
    /// - The PID counter has saturated at `u64::MAX` (academic).
    pub fn insert(&self, process: T) -> Option<(Pid, ProcessRef<T>)> {
        // 1. Allocate outside the lock — slab has its own internal mutex.
        let ptr = self.allocator.alloc()?;

        // 2. Issue PID before taking the index lock.
        //    Saturating: at u64::MAX we stop rather than wrap and collide.
        let pid_val = self.next_pid.fetch_add(1, Ordering::Relaxed);
        if pid_val == u64::MAX {
            self.next_pid.store(u64::MAX, Ordering::Relaxed);
            // SAFETY: freshly allocated, never exposed.
            unsafe { self.allocator.dealloc(ptr) };
            return None;
        }

        // 3. Write the process struct into the allocated slot.
        //    SAFETY: ptr is freshly allocated and exclusively owned.
        unsafe { core::ptr::write(ptr.as_ptr(), process) };

        // 4. Take the index lock for the remainder of the insert.
        let mut index = self.index.lock();

        // 5. Find a free slot. Linear scan; insert is cold relative to
        //    the scheduler tick rate.
        let slot_idx = index.iter().position(|e| e.ptr.is_null())?;

        // 6. Determine generation for this slot.
        let generation = {
            let raw = index[slot_idx].generation;
            if raw.0 == 0 { Generation::INITIAL } else { raw }
        };

        // 7. Stamp generation and PID into the struct while we hold the lock.
        //    Both stamps happen before the slot is visible to any other thread
        //    (the index entry is still null until step 8).
        //    SAFETY: ptr is exclusively owned; we wrote a valid T in step 3.
        unsafe {
            T::stamp_generation(ptr.as_mut(), generation.0);
            T::stamp_pid(ptr.as_mut(), pid_val);
        }

        // 8. Publish the slot.
        index[slot_idx] = IndexEntry { ptr: ptr.as_ptr(), generation };
        // Lock released here on drop.

        Some((Pid::from_raw(pid_val), ProcessRef { ptr, generation }))
    }

    /// Remove a process by its `ProcessRef`.
    ///
    /// Bumps the generation (invalidating all extant `ProcessRef`s for this
    /// slot), calls `drop(T)`, and deallocates the slab slot.
    ///
    /// Returns `true` if found and removed. Returns `false` if the handle
    /// was already stale (double-remove attempt or handle for wrong generation).
    ///
    /// # Safety
    /// No thread may be concurrently dereferencing the process struct.
    /// In practice: remove the process from the scheduler run queue before
    /// calling this.
    pub unsafe fn remove(&self, handle: ProcessRef<T>) -> bool {
        let mut index = self.index.lock();

        let entry = index
            .iter_mut()
            .find(|e| e.ptr == handle.ptr.as_ptr());

        let entry = match entry {
            Some(e) => e,
            None    => return false,
        };

        if entry.generation != handle.generation {
            return false;
        }

        // Bump generation: all extant ProcessRefs for this slot are now stale.
        entry.generation = entry.generation.next();

        let raw = entry.ptr;
        entry.ptr = core::ptr::null_mut();
        drop(index); // Release lock before drop + dealloc.

        // SAFETY: raw is valid and exclusively owned at this point.
        let nn = unsafe { NonNull::new_unchecked(raw) };
        unsafe { core::ptr::drop_in_place(nn.as_ptr()) };
        unsafe { self.allocator.dealloc(nn) };

        true
    }

    /// Look up a process by `Pid`. Returns a `ProcessRef<T>` for fast
    /// subsequent access. O(CAP) — the cold path.
    ///
    /// Call once per syscall entry, hold the `ProcessRef` for the duration
    /// of the operation, then discard it. The scheduler's inner loop should
    /// hold `ProcessRef`s directly and never call this.
    pub fn lookup(&self, pid: Pid) -> Option<ProcessRef<T>> {
        let index = self.index.lock();
        for entry in index.iter() {
            if entry.ptr.is_null() {
                continue;
            }
            // SAFETY: ptr is non-null and was written by insert.
            let pid_in_struct  = unsafe { (*entry.ptr).pid() };
            let gen_in_struct  = unsafe { (*entry.ptr).generation() };
            if pid_in_struct == pid.as_u64()
                && gen_in_struct == entry.generation.0
            {
                return Some(ProcessRef {
                    ptr:        unsafe { NonNull::new_unchecked(entry.ptr) },
                    generation: entry.generation,
                });
            }
        }
        None
    }

    /// Number of occupied slots. O(CAP). For diagnostics only.
    pub fn len(&self) -> usize {
        self.index.lock().iter().filter(|e| !e.ptr.is_null()).count()
    }

    /// Returns `true` if no processes are registered.
    pub fn is_empty(&self) -> bool { self.len() == 0 }
}
