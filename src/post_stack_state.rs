// v0.1.2
//! Global state cell for data that must survive the kernel stack switch.
//!
//! `kernel_main` runs on the Limine-provided boot stack. After memory
//! initialization it allocates a permanent kernel stack and switches to it
//! via [`crate::memory::stack::switch_stack`]. Because `switch_stack` takes
//! a bare `fn() -> !` (not a closure), no local variables from `kernel_main`
//! can be carried across — they live on the old stack, which is abandoned.
//!
//! Any data that the post-switch entry point needs must be stored here before
//! the switch and consumed after.
//!
//! # Usage
//!
//! ```rust
//! // In kernel_main, before switch_stack():
//! post_stack_state::store(PostStackState {
//!     example_field: value,
//! });
//!
//! unsafe { switch_stack(kstack.top, kernel_main_continued) }
//!
//! // In kernel_main_continued():
//! fn kernel_main_continued() -> ! {
//!     let state = post_stack_state::take();
//!     // use state.example_field ...
//! }
//! ```
//!
//! # Design constraints
//!
//! - `PostStackState` must be `Send` (kernel is single-core for now, but the
//!   constraint makes the assumption explicit and safe).
//! - All fields must be valid across a raw stack pointer change — no references
//!   into the old stack frame, no `NonNull` pointers to stack-allocated data.
//! - `take()` panics on a second call; `store()` panics if called twice without
//!   an intervening `take()`. Both are programming errors, not runtime conditions.
//!
//! # Current fields
//!

use core::sync::atomic::{AtomicBool, Ordering};
use spin::Mutex;

// ── PostStackState ────────────────────────────────────────────────────────────

/// Data carried from the pre-switch half of `kernel_main` to
/// `kernel_main_continue`.
///
/// All fields are plain integers — no pointers into old stack frames.
pub struct PostStackState {
    /// Physical address of the ACPI RSDP table, if Limine provided one.
    pub rsdp_phys: Option<u64>,
    /// Reclaimable region skipped by `release()` because it held the boot RSP.
    /// Stored as `(hhdm_virt_base, page_count)`. Added to buddy after switch.
    pub boot_stack_region: Option<(u64, usize)>,
}

// SAFETY: all fields are plain integers. No thread actually runs concurrently
// at this stage; the bound makes the intent explicit.
unsafe impl Send for PostStackState {}

// ── Storage cell ──────────────────────────────────────────────────────────────

static STORED: AtomicBool  = AtomicBool::new(false);
static TAKEN:  AtomicBool  = AtomicBool::new(false);
static CELL:   Mutex<Option<PostStackState>> = Mutex::new(None);

/// Store `state` for retrieval by the post-switch entry point.
///
/// # Panics
///
/// Panics if called more than once without an intervening `take`.
pub fn store(state: PostStackState) {
    assert!(
        !STORED.swap(true, Ordering::SeqCst),
        "post_stack_state::store() called twice without an intervening take()"
    );
    *CELL.lock() = Some(state);
}

/// Take the stored state. May only be called once, after `store`.
///
/// # Panics
///
/// Panics if `store` has not been called, or if `take` has already been called.
pub fn take() -> PostStackState {
    assert!(
        STORED.load(Ordering::SeqCst),
        "post_stack_state::take() called before store()"
    );
    assert!(
        !TAKEN.swap(true, Ordering::SeqCst),
        "post_stack_state::take() called more than once"
    );
    CELL.lock()
        .take()
        .expect("post_stack_state: cell was None after store() — this is a bug")
}
