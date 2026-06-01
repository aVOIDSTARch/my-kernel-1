# Seastar — Process, Thread, and Context Data Models

> **Audience:** A Claude instance integrating the `seastar` crate into an existing Rust
> x86_64 kernel that already has the `interrupts/` subsystem and a GDT/TSS in place.
> `seastar` is pure data. It does not touch hardware, does not perform context switches,
> and does not implement scheduling policy. It defines the types that `cephalopod` (the
> scheduler) and the kernel proper share. If you find yourself reaching for a port read or
> an MSR write inside this crate, you are in the wrong crate.

-----

## What This Crate Is

`seastar` owns the canonical definitions of every type that describes a schedulable unit
of execution and its saved hardware state. Its consumers are:

- **The kernel proper** — creates and destroys `Process` and `Thread` instances.
- **`cephalopod`** — reads `ProcessState`, reads and writes `Context`, walks run queues.
- **The interrupt subsystem** — reads `Thread::kernel_stack_top` to update `TSS.rsp0`
  on every context switch.

The crate is `#![no_std]`. It depends on `alloc` for owned collections inside `Process`
(thread lists, file descriptor tables). If your kernel has no global allocator yet, stub
those fields out behind a cargo feature and add them when the heap is ready.

-----

## Crate Layout

```
seastar/
├── Cargo.toml
├── src/
│   ├── lib.rs          — crate root; re-exports public surface
│   ├── ids.rs          — ProcessId, ThreadId newtypes
│   ├── state.rs        — ProcessState, ThreadState enums
│   ├── context.rs      — Context (saved register file + stack pointer)
│   ├── stack.rs        — KernelStack (owned stack allocation + guard page logic)
│   ├── thread.rs       — Thread struct
│   ├── process.rs      — Process struct
│   ├── priority.rs     — Priority newtype and scheduling metadata
│   └── flags.rs        — ProcessFlags, ThreadFlags bitfields
```

-----

## Cargo.toml

```toml
[package]
name    = "seastar"
version = "0.1.0"
edition = "2021"

[dependencies]
bitflags  = "2.4"
spin      = "0.9"

[features]
default = ["alloc"]
alloc   = []          # gate Vec/BTreeMap fields; disable on heap-less targets
```

`bitflags` generates the `ProcessFlags` and `ThreadFlags` implementations.
`spin` provides the `Mutex` wrapping mutable fields that may be accessed from interrupt
context. No `lazy_static`, no `x86_64` crate dependency — those belong to consumers.

-----

## `ids.rs` — Identity Types

Process and thread IDs are newtypes, not raw integers. Passing a `u64` where a
`ProcessId` is expected is a compile error, not a silent bug.

```rust
// src/ids.rs
#![allow(clippy::new_without_default)]

use core::sync::atomic::{AtomicU64, Ordering};

static NEXT_PID: AtomicU64 = AtomicU64::new(1); // 0 reserved for the idle process
static NEXT_TID: AtomicU64 = AtomicU64::new(1);

/// Unique identifier for a process (address space owner).
/// PID 0 is the idle process; it is never allocated by `ProcessId::new()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct ProcessId(u64);

impl ProcessId {
    /// Allocate the next available PID. Monotonically increasing; never reused.
    /// Wrapping at u64::MAX is not handled — if your kernel creates 2^64 processes
    /// something has gone more wrong than an overflow check can fix.
    pub fn new() -> Self {
        Self(NEXT_PID.fetch_add(1, Ordering::Relaxed))
    }

    /// The idle process PID. Not allocatable via `new()`.
    pub const IDLE: Self = Self(0);

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl core::fmt::Display for ProcessId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "PID({})", self.0)
    }
}

/// Unique identifier for a thread (schedulable unit within a process).
/// TID 0 is the idle thread; it is never allocated by `ThreadId::new()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct ThreadId(u64);

impl ThreadId {
    pub fn new() -> Self {
        Self(NEXT_TID.fetch_add(1, Ordering::Relaxed))
    }

    pub const IDLE: Self = Self(0);

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl core::fmt::Display for ThreadId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "TID({})", self.0)
    }
}
```

**Do not** add a `from_u64` constructor or `impl From<u64>`. The newtypes are only useful
if arbitrary integers cannot be laundered into them.

-----

## `state.rs` — State Machines

```rust
// src/state.rs

/// The lifecycle state of a process.
///
/// Transitions:
///   Created → Ready (when initial thread is scheduled)
///   Ready ↔ Running (scheduler decision)
///   Running → Blocked (syscall, I/O wait, mutex)
///   Blocked → Ready (event delivery)
///   Running → Zombie (exit() called; resources not yet reaped)
///   Zombie → (destroyed, no further state)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    /// Allocated but not yet on any run queue. Initial state.
    Created,
    /// On a run queue; eligible to run.
    Ready,
    /// Currently executing on a CPU core.
    Running,
    /// Waiting for an event. Not on any run queue.
    Blocked(BlockedReason),
    /// `exit()` called. Waiting for parent to reap exit status.
    Zombie,
}

/// The lifecycle state of an individual thread.
///
/// Mirrors `ProcessState` but at thread granularity. A process is `Running`
/// if any of its threads is `Running`; `Blocked` if all threads are `Blocked`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    Created,
    Ready,
    Running,
    Blocked(BlockedReason),
    /// Thread has exited. Not yet joined by another thread.
    Dead,
}

/// Why a thread is blocked. Passed to the scheduler so it can efficiently
/// wake threads on the correct event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockedReason {
    /// Waiting for a specific duration (sleep, timeout).
    Sleep,
    /// Waiting on a mutex, semaphore, or condition variable.
    Synchronisation,
    /// Waiting for I/O completion (disk, network, keyboard).
    Io,
    /// Waiting for a child process to exit.
    WaitChild,
    /// Reason not categorised; scheduler treats as opaque.
    Other,
}
```

-----

## `context.rs` — Saved Hardware State

This is the most architecture-specific type in `seastar`. It holds exactly the register
state that the context switcher in `cephalopod` must save and restore. The layout here
must match what the assembly switch stub pushes and pops — if you change one, you must
change both.

```rust
// src/context.rs

/// Saved x86_64 register state for a suspended thread.
///
/// Only callee-saved registers appear here. Caller-saved registers
/// (rax, rcx, rdx, rsi, rdi, r8–r11) are the responsibility of the
/// interrupted code by the System V ABI — they are either saved by the
/// compiler on the thread's own stack, or considered dead at the switch point.
///
/// `rsp` is the kernel stack pointer at the moment the thread was suspended.
/// On resume, the context switcher loads this value into RSP and then pops
/// the remaining fields in the order they were pushed.
///
/// `rip` is not stored directly. Instead, the context switcher uses the
/// return address on the kernel stack — a `ret` instruction at the end of
/// the switch stub jumps to wherever the thread was last interrupted.
/// A newly created thread must have its entry point placed as a synthetic
/// return address at the top of its kernel stack before it is first scheduled.
///
/// # Alignment
/// `repr(C)` is required. The context switcher is assembly that accesses
/// fields at fixed offsets. Do not reorder fields without updating the
/// assembly in `cephalopod`.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Context {
    /// Kernel stack pointer at the point of suspension.
    /// This is the first field because the switch stub saves RSP before
    /// any other register, and loads it first on resume.
    pub rsp: u64,

    // Callee-saved general-purpose registers (System V AMD64 ABI).
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub rbx: u64,
    pub rbp: u64, // frame pointer; may be omitted if -Cno-frame-pointers, but keep for now
}

impl Context {
    /// Produce a zeroed context. Only valid as a placeholder before the
    /// context is properly initialised by the thread creation path.
    /// A thread with a zeroed context must never be scheduled.
    pub const fn zeroed() -> Self {
        Self {
            rsp: 0,
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            rbx: 0,
            rbp: 0,
        }
    }

    /// Construct the initial context for a newly created thread.
    ///
    /// `stack_top` is the virtual address of the top of the thread's kernel
    /// stack (highest address; x86 stacks grow downward). The entry point
    /// address is placed as a synthetic return address at `stack_top - 8`
    /// so that the switch stub's `ret` jumps to it on first schedule.
    ///
    /// # Safety
    /// `stack_top` must point to mapped, writable kernel memory. The 8 bytes
    /// at `stack_top - 8` are written immediately by this function.
    pub unsafe fn new_thread(stack_top: u64, entry_point: u64) -> Self {
        // Place the entry point as a fake return address on the stack.
        let rsp = stack_top - 8;
        let ret_addr_slot = rsp as *mut u64;
        unsafe { ret_addr_slot.write(entry_point); }

        Self {
            rsp,
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            rbx: 0,
            rbp: 0,
        }
    }
}
```

-----

## `stack.rs` — Kernel Stack Ownership

Each thread owns its kernel stack. `KernelStack` wraps the allocation and exposes the
stack top address that `Context::new_thread` and `TSS.rsp0` both need.

```rust
// src/stack.rs

/// Default kernel stack size per thread: 16 KiB.
/// Increase this if kernel code is stack-heavy (deep recursion in VFS, etc.).
/// Stack overflow within a kernel thread is not reliably detectable without
/// a guard page — see `KernelStack::with_guard`.
pub const KERNEL_STACK_SIZE: usize = 4096 * 4; // 16 KiB

/// An owned kernel stack allocation.
///
/// The stack is allocated as a `Box<[u8]>` and is freed when this struct
/// is dropped. The thread that owns this stack must not be running when
/// `KernelStack` is dropped — dropping a live thread's stack is undefined
/// behaviour that no type system can fully prevent at this layer.
pub struct KernelStack {
    /// The raw allocation. Held to own the memory; not accessed directly.
    #[cfg(feature = "alloc")]
    _storage: alloc::boxed::Box<[u8]>,
    /// Virtual address of the byte one past the top of the stack
    /// (i.e., the initial RSP value before any pushes).
    /// x86 stacks grow downward; this is the highest address.
    stack_top: u64,
}

#[cfg(feature = "alloc")]
impl KernelStack {
    /// Allocate a kernel stack of `KERNEL_STACK_SIZE` bytes.
    /// The allocation is 16-byte aligned (satisfying SSE requirements).
    ///
    /// No guard page is installed. For guard page support, use
    /// `KernelStack::with_guard`, which requires cooperation from your
    /// virtual memory subsystem.
    pub fn new() -> Self {
        use alloc::vec;
        let storage = vec![0u8; KERNEL_STACK_SIZE].into_boxed_slice();
        // stack_top is the address just past the end of the allocation,
        // which is the initial RSP (first push will decrement RSP by 8
        // before writing, landing within the allocation).
        let stack_top = storage.as_ptr() as u64 + KERNEL_STACK_SIZE as u64;
        Self { _storage: storage, stack_top }
    }

    /// Virtual address of the stack top (highest address).
    /// Pass this to `Context::new_thread` and to the TSS `rsp0` updater.
    pub fn top(&self) -> u64 {
        self.stack_top
    }
}

#[cfg(feature = "alloc")]
impl core::fmt::Debug for KernelStack {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "KernelStack {{ top: {:#018x} }}", self.stack_top)
    }
}
```

> **Guard pages:** Mapping the page immediately below the stack allocation as
> non-present would turn a stack overflow into a clean page fault rather than
> silent memory corruption. This requires your VMM to unmap a page at a specific
> physical address — out of scope for `seastar` itself, but the integration point
> is `KernelStack::new()`. When your VMM is ready, add a `with_guard(vmm: &mut Vmm)`
> constructor here.

-----

## `priority.rs` — Scheduling Metadata

```rust
// src/priority.rs

/// Thread priority. Lower numeric value = higher scheduling priority.
/// Range 0–255. Priority 0 is reserved for the idle thread.
/// Real-time threads: 1–31. Normal threads: 32–231. Idle-class: 232–254.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Priority(u8);

impl Priority {
    pub const IDLE:        Self = Self(0);
    pub const REALTIME_HI: Self = Self(1);
    pub const REALTIME_LO: Self = Self(31);
    pub const NORMAL:      Self = Self(128);
    pub const NORMAL_HI:   Self = Self(64);
    pub const NORMAL_LO:   Self = Self(191);
    pub const BACKGROUND:  Self = Self(232);

    pub fn new(value: u8) -> Option<Self> {
        // 0 is reserved for idle; callers cannot create it directly.
        if value == 0 { None } else { Some(Self(value)) }
    }

    pub fn as_u8(self) -> u8 { self.0 }
}

impl Default for Priority {
    fn default() -> Self { Self::NORMAL }
}

/// Accumulated scheduling statistics for a thread.
/// Updated by `cephalopod` on each context switch.
/// `seastar` defines the shape; `cephalopod` owns the update logic.
#[derive(Debug, Default, Clone, Copy)]
pub struct SchedulingStats {
    /// Total CPU time consumed, in scheduler ticks.
    pub cpu_ticks: u64,
    /// Number of times this thread has been context-switched in.
    pub context_switches: u64,
    /// Number of times this thread voluntarily yielded the CPU.
    pub voluntary_yields: u64,
}
```

-----

## `flags.rs` — Bitfield Types

```rust
// src/flags.rs
use bitflags::bitflags;

bitflags! {
    /// Per-process flags. Accessed from interrupt context; use atomic operations
    /// or a spin lock when modifying from multiple contexts.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ProcessFlags: u32 {
        /// Process is a kernel process (ring 0 only; no user address space).
        const KERNEL_PROCESS   = 1 << 0;
        /// Process has called exit() but has not been reaped.
        const EXITING          = 1 << 1;
        /// Process is being traced (e.g., via a future ptrace equivalent).
        const TRACED           = 1 << 2;
        /// Process has received a signal that has not yet been delivered.
        const SIGNAL_PENDING   = 1 << 3;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ThreadFlags: u32 {
        /// Thread is the main thread of its process.
        const MAIN_THREAD      = 1 << 0;
        /// Thread has called exit() or been cancelled.
        const EXITING          = 1 << 1;
        /// FPU/SIMD state is stale; must restore before first FPU instruction.
        /// Used by the lazy FPU switch path in `cephalopod`.
        const FPU_STATE_DIRTY  = 1 << 2;
        /// Thread is currently executing in a kernel syscall path.
        const IN_SYSCALL       = 1 << 3;
    }
}
```

-----

## `thread.rs` — Thread Struct

```rust
// src/thread.rs
use crate::{
    context::Context,
    flags::ThreadFlags,
    ids::{ProcessId, ThreadId},
    priority::{Priority, SchedulingStats},
    stack::KernelStack,
    state::ThreadState,
};

/// A schedulable unit of execution within a process.
///
/// One `Thread` exists per OS thread. `cephalopod` holds references to
/// `Thread` instances in its run queues. The kernel proper creates and
/// destroys them; `cephalopod` only mutates `state`, `context`, and `stats`.
///
/// # Field access from interrupt context
/// `context` and `state` may be read or written from the timer IRQ handler
/// (interrupt context). All such accesses must go through the `spin::Mutex`
/// wrappers or use atomic operations. Do not add bare mutable fields that
/// can race with the scheduler tick.
pub struct Thread {
    pub id:      ThreadId,
    pub owner:   ProcessId,
    pub flags:   spin::Mutex<ThreadFlags>,
    pub state:   spin::Mutex<ThreadState>,
    pub context: spin::Mutex<Context>,
    pub priority: Priority,
    pub stats:   spin::Mutex<SchedulingStats>,

    /// Kernel stack. Owned by this thread for its entire lifetime.
    /// `stack.top()` is the value written to `TSS.rsp0` whenever this
    /// thread is scheduled onto a CPU core.
    pub stack: KernelStack,
}

impl Thread {
    /// Create a new thread belonging to `owner`, with its entry point at
    /// `entry_point` (a kernel virtual address).
    ///
    /// # Safety
    /// `entry_point` must be a valid kernel function pointer. The thread is
    /// not scheduled until `cephalopod` places it on a run queue.
    #[cfg(feature = "alloc")]
    pub unsafe fn new(owner: ProcessId, entry_point: u64, priority: Priority) -> Self {
        let stack = KernelStack::new();
        let context = unsafe { Context::new_thread(stack.top(), entry_point) };

        Self {
            id:       ThreadId::new(),
            owner,
            flags:    spin::Mutex::new(ThreadFlags::MAIN_THREAD),
            state:    spin::Mutex::new(ThreadState::Created),
            context:  spin::Mutex::new(context),
            priority,
            stats:    spin::Mutex::new(SchedulingStats::default()),
            stack,
        }
    }

    /// The stack top address to write into `TSS.rsp0` when this thread
    /// is scheduled. Called by `cephalopod` on every context switch.
    pub fn kernel_stack_top(&self) -> u64 {
        self.stack.top()
    }
}

impl core::fmt::Debug for Thread {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Thread")
            .field("id",    &self.id)
            .field("owner", &self.owner)
            .field("state", &*self.state.lock())
            .field("priority", &self.priority)
            .finish()
    }
}
```

-----

## `process.rs` — Process Struct

```rust
// src/process.rs
use crate::{
    flags::ProcessFlags,
    ids::ProcessId,
    state::ProcessState,
    thread::Thread,
};

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// An address space owner. Contains one or more threads.
///
/// The `cr3` field holds the physical address of the root page table
/// (PML4 on x86_64). This is what the context switcher loads into CR3
/// when switching between processes. Threads within the same process
/// share a `cr3` — context switching between sibling threads skips the
/// CR3 load (and the associated TLB flush) entirely.
///
/// `cr3 = 0` is a sentinel meaning "kernel process; use the kernel's
/// page table, never switch CR3". The idle process uses this.
pub struct Process {
    pub id:     ProcessId,
    pub state:  spin::Mutex<ProcessState>,
    pub flags:  spin::Mutex<ProcessFlags>,

    /// Physical address of the PML4 page table root.
    /// 0 for kernel processes (no address space switch required).
    pub cr3: u64,

    /// All threads belonging to this process.
    /// The first entry is always the main thread.
    #[cfg(feature = "alloc")]
    pub threads: spin::Mutex<Vec<Thread>>,

    /// Exit status, valid only when `state` is `ProcessState::Zombie`.
    pub exit_code: core::sync::atomic::AtomicI32,
}

#[cfg(feature = "alloc")]
impl Process {
    /// Create a new kernel process (ring 0 only, no user address space).
    /// `entry_point` becomes the main thread's initial instruction pointer.
    ///
    /// # Safety
    /// `entry_point` must be a valid kernel virtual address pointing to a
    /// function that never returns (or calls `exit()` before returning).
    pub unsafe fn new_kernel(entry_point: u64) -> Self {
        use crate::priority::Priority;
        use crate::flags::ThreadFlags;

        let id = ProcessId::new();
        let mut main_thread = unsafe {
            Thread::new(id, entry_point, Priority::NORMAL)
        };
        // Explicitly mark as main thread (Thread::new sets this by default,
        // but be explicit for clarity).
        *main_thread.flags.lock() |= ThreadFlags::MAIN_THREAD;

        Self {
            id,
            state:     spin::Mutex::new(ProcessState::Created),
            flags:     spin::Mutex::new(ProcessFlags::KERNEL_PROCESS),
            cr3:       0,
            threads:   spin::Mutex::new(alloc::vec![main_thread]),
            exit_code: core::sync::atomic::AtomicI32::new(0),
        }
    }

    /// The physical CR3 value this process needs when scheduled.
    /// Returns `None` for kernel processes (no CR3 switch necessary).
    pub fn cr3_for_switch(&self) -> Option<u64> {
        if self.cr3 == 0 { None } else { Some(self.cr3) }
    }

    /// Count of live (non-dead) threads.
    pub fn live_thread_count(&self) -> usize {
        use crate::state::ThreadState;
        self.threads.lock()
            .iter()
            .filter(|t| *t.state.lock() != ThreadState::Dead)
            .count()
    }
}

impl core::fmt::Debug for Process {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Process")
            .field("id",    &self.id)
            .field("state", &*self.state.lock())
            .field("cr3",   &format_args!("{:#018x}", self.cr3))
            .finish()
    }
}
```

-----

## `lib.rs` — Public Surface

```rust
// src/lib.rs
#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod context;
pub mod flags;
pub mod ids;
pub mod priority;
pub mod process;
pub mod stack;
pub mod state;
pub mod thread;

// Convenience re-exports for the most common types.
pub use context::Context;
pub use ids::{ProcessId, ThreadId};
pub use priority::Priority;
pub use process::Process;
pub use state::{BlockedReason, ProcessState, ThreadState};
pub use thread::Thread;
```

-----

## Integration Guide

### Step 1: Add `seastar` to workspace

In your kernel workspace `Cargo.toml`:

```toml
[workspace]
members = [
    "kernel",
    "seastar",
    "cephalopod",   # not yet; add when you build the scheduler
]
```

In `kernel/Cargo.toml` and `cephalopod/Cargo.toml` (when it exists):

```toml
[dependencies]
seastar = { path = "../seastar" }
```

### Step 2: Verify your allocator

`seastar` with the default `alloc` feature requires a global allocator before you can
call `Process::new_kernel` or `Thread::new`. The allocator must be initialised before
any `seastar` type that allocates is constructed. If you are using `mimalloc` or
`jemalloc` as your kernel heap (per `memory_allocators.md`), ensure `#[global_allocator]`
is set in `kernel/src/main.rs` before `kernel_main` creates any processes.

If your heap is not yet ready, add `default-features = false` to the `seastar` dependency
and do not call any `#[cfg(feature = "alloc")]`-gated functions until it is.

### Step 3: TSS `rsp0` update site

Every context switch must update `TSS.rsp0` to point to the incoming thread’s kernel stack
top. This is the only integration point between `seastar` and your existing GDT/TSS code.

In your GDT module, expose a function:

```rust
// src/gdt.rs
pub fn set_kernel_stack(rsp0: u64) {
    // The x86_64 crate's TSS does not expose rsp0 mutably through a safe API.
    // This requires a direct write through a raw pointer into the TSS structure.
    unsafe {
        let tss_ptr = &raw mut TSS as *mut x86_64::structures::tss::TaskStateSegment;
        (*tss_ptr).privilege_stack_table[0] =
            x86_64::VirtAddr::new(rsp0);
    }
}
```

In `cephalopod`, after switching `Context` (loading new RSP), call:

```rust
kernel::gdt::set_kernel_stack(incoming_thread.kernel_stack_top());
```

This must happen before the `iretq` that returns to the new thread, or before the new
thread’s first interrupt would be taken with a stale `rsp0`. The safe ordering is:
update `TSS.rsp0`, then swap RSP in the switch stub.

### Step 4: CR3 switching

When switching between two threads belonging to different processes, load the incoming
process’s `cr3` value into the CR3 register. When switching between threads in the same
process, skip this — a redundant CR3 load flushes the TLB for no benefit.

In `cephalopod`’s switch stub:

```rust
if let Some(cr3) = incoming_process.cr3_for_switch() {
    unsafe {
        core::arch::asm!(
            "mov cr3, {}",
            in(reg) cr3,
            options(nostack, preserves_flags)
        );
    }
}
```

### Step 5: FPU state

`ThreadFlags::FPU_STATE_DIRTY` is the hook for lazy FPU switching. The interrupt subsystem
document explicitly notes that the `#NM` (Device Not Available) handler is a panic stub
pending implementation of lazy FPU context switching.

The intended flow:

1. On context switch out, set `FPU_STATE_DIRTY` on the outgoing thread and set `CR0.TS`.
1. On context switch in, do **not** restore FPU state immediately.
1. When the incoming thread executes its first FPU instruction, `#NM` fires.
1. In the `#NM` handler: clear `CR0.TS`, restore the thread’s FPU state from a
   `FpuState` buffer (not yet defined in `seastar`; add it to `thread.rs` when needed),
   clear `FPU_STATE_DIRTY`.

Do not implement this until you have a working basic context switcher. It is an
optimisation, not a correctness requirement for kernel threads that do not use FPU.

### Step 6: Idle process

The idle process is the fallback when `cephalopod` has no runnable thread. Create it
before the scheduler starts:

```rust
// In kernel_main, after heap init, before interrupts::init():
let idle = unsafe {
    seastar::Process::new_kernel(idle_thread_entry as u64)
};
// Pass to cephalopod as its designated idle process.
cephalopod::set_idle_process(idle);
```

The idle thread’s entry function must be a `-> !` loop:

```rust
fn idle_thread_entry() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}
```

-----

## What `seastar` Does Not Own

These are explicit scope boundaries:

**No context switch assembly.** The `switch_to` stub that saves and restores registers
and swaps RSP belongs in `cephalopod`. `seastar` defines `Context`’s field layout; the
assembly must match it.

**No run queues.** `Vec<Thread>` inside `Process` is a thread roster, not a run queue.
`cephalopod` maintains its own run queues that hold references or IDs into `seastar` types.

**No page table management.** `cr3` is a u64 physical address. Allocating, populating,
and freeing page tables belongs to your VMM subsystem.

**No signal delivery.** `ProcessFlags::SIGNAL_PENDING` is a marker. The signal queue and
delivery mechanism are not here.

**No file descriptor tables.** Deliberately omitted from this version. Add as a field in
`Process` behind a feature flag when your VFS layer exists.

**No timing.** `SchedulingStats::cpu_ticks` is a counter. What constitutes a tick, and
what the real-time value of a tick is, belongs to `cephalopod` and your timer subsystem.

-----

## Type Relationship Diagram

```
Process
  ├── ProcessId          (ids.rs)
  ├── ProcessState       (state.rs)
  ├── ProcessFlags       (flags.rs)
  ├── cr3: u64           (raw; VMM-assigned)
  └── threads: Vec<Thread>
        └── Thread
              ├── ThreadId         (ids.rs)
              ├── ProcessId        (owner back-reference)
              ├── ThreadState      (state.rs)
              ├── ThreadFlags      (flags.rs)
              ├── Context          (context.rs)
              │     ├── rsp: u64
              │     ├── r15–r12: u64
              │     ├── rbx: u64
              │     └── rbp: u64
              ├── Priority         (priority.rs)
              ├── SchedulingStats  (priority.rs)
              └── KernelStack      (stack.rs)
                    └── stack_top: u64  ← written to TSS.rsp0 on switch
```

-----

*Reflects the state of the kernel project through mid-2025. Update this document when
FPU state, signal handling, or user-space thread support is added to `seastar`.*