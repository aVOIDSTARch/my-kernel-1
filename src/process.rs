//! Kernel process table wiring.
//!
//! Connects `seastar::ProcessTable` to `SlabCache<Process>` and exposes
//! a global accessor. No data model or assembly lives here.
//!
//! ## Invariants
//!
//! - `init()` must be called after `memory::heap::init()`.
//! - `init()` must be called before interrupts are enabled and before any
//!   code calls `get()`.
//! - `get()` panics if called before `init()`.

use spin::Once;
use core::ptr::NonNull;

use crate::arch::X86IrqControl;
use abalone::slab::SlabCache;
use seastar::{
    Process,
    table::{Allocator, ProcessTable},
};

// ── Compile-time size/alignment guard ─────────────────────────────────────────

// Verify Process is a valid slab element before any allocation occurs.
// This fires a compile error (not a runtime panic) if Process is too small
// or over-aligned for the slab allocator. The constant is never read;
// its evaluation is the check.
const _: () = abalone::slab::assert_slab_compatible::<Process>();

// ── SlabBacked allocator ──────────────────────────────────────────────────────

struct SlabBacked(SlabCache<Process>);

impl Allocator<Process> for SlabBacked {
    fn alloc(&self) -> Option<NonNull<Process>> {
        self.0.alloc()
    }

    unsafe fn dealloc(&self, ptr: NonNull<Process>) {
        // SAFETY: contract forwarded from ProcessTable::remove.
        unsafe { self.0.dealloc(ptr) }
    }
}

// ── Global table ──────────────────────────────────────────────────────────────

/// Maximum concurrent processes. 1024 slots occupy ≈ 24 KiB in .bss.
/// Increase if the kernel needs to support more simultaneous processes.
const PROCESS_CAP: usize = 1024;

type KernelTable = ProcessTable<Process, SlabBacked, PROCESS_CAP, X86IrqControl>;

static TABLE: Once<KernelTable> = Once::new();

/// Initialise the process table. Call once, after `memory::heap::init()`,
/// before enabling interrupts.
pub fn init() {
    TABLE.call_once(|| {
        // slab_order = 0: one 4 KiB page per slab.
        // If Process grows large enough that fewer than 2 instances fit in
        // 4 KiB, increase to slab_order = 1 (8 KiB slabs).
        // The compile-time guard above will not catch this case automatically;
        // monitor slab utilization if Process gains large fields.
        ProcessTable::new(SlabBacked(SlabCache::new(0)))
    });
}

/// Return a reference to the global process table.
///
/// # Panics
/// Panics if `process::init()` has not been called.
pub fn get() -> &'static KernelTable {
    TABLE.get().expect("process::init() not called before process::get()")
}
