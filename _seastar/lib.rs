//! # seastar — Process, Thread, and Context Data Models
//!
//! Pure data crate. No hardware access, no context switching, no scheduling
//! policy. Defines the types that `cephalopod` (scheduler) and the kernel
//! proper share.
//!
//! ## Crate structure
//!
//! ```text
//! seastar
//! ├── ids.rs      — ProcessId, ThreadId newtypes
//! ├── state.rs    — ProcessState, ThreadState, BlockedReason enums
//! ├── context.rs  — Context (saved register file + kernel RSP)
//! ├── stack.rs    — KernelStack (owned stack allocation)
//! ├── thread.rs   — Thread struct
//! ├── process.rs  — Process struct + table trait impls
//! ├── priority.rs — Priority, SchedulingStats
//! ├── flags.rs    — ProcessFlags, ThreadFlags bitfields
//! └── table.rs    — ProcessTable, Pid, ProcessRef, Allocator + traits
//! ```
//!
//! ## Dependencies
//!
//! - `pincer` — `IrqMutex`, `SpinMutex`, `AtomicCell`, `IrqControl`
//! - `bitflags` — `ProcessFlags`, `ThreadFlags`
//! - `alloc` (feature-gated) — `Vec`, `Box`

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod context;
pub mod flags;
pub mod ids;
pub mod priority;
pub mod process;
pub mod stack;
pub mod state;
pub mod table;
pub mod thread;

// ── Convenience re-exports ────────────────────────────────────────────────────

pub use context::Context;
pub use ids::{ProcessId, ThreadId};
pub use priority::Priority;
pub use process::Process;
pub use state::{BlockedReason, ProcessState, ThreadState};
pub use thread::Thread;

// Table surface — re-export everything a kernel wiring module needs.
pub use table::{
    Allocator,
    HasGeneration,
    HasPid,
    Pid,
    ProcessRef,
    ProcessTable,
    StampGeneration,
    StampPid,
};
