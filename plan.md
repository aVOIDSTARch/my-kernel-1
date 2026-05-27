# Kernel → Terminal Shell: Multiphase Roadmap

---

## Phase 1 — Memory Management Foundation
*Pre-requisite for everything. Without dynamic allocation, no complex data structures are possible.*

- **Virtual Memory Manager (VMM)**: page table walker, `map`/`unmap`/`remap` API over Limine's initial page tables
- **Kernel Heap**: implement a heap allocator (linked-list or slab); wire in Rust's global allocator trait
- **`alloc` crate support**: enable `Box`, `Vec`, `String`, `BTreeMap` etc. throughout the kernel
- **MMIO abstraction**: typed, page-mapped windows into device register regions

---

## Phase 2 — Interrupt & Timer Infrastructure
*Keyboard events arrive via IRQs; timers drive any future scheduling.*

- **PIC → APIC migration**: detect APIC via ACPI/CPUID, remap/mask legacy PIC, enable LAPIC
- **IRQ dispatch table**: vector-to-handler registry, EOI handling, spurious IRQ suppression
- **PIT / APIC timer**: periodic tick at a fixed rate; `uptime_ms()` and `sleep_ms()` primitives
- **Software exceptions**: complete fault handlers (page fault, GPF, double fault with IST)

---

## Phase 3 — Input Drivers
*The shell needs a way to receive characters from the user.*

- **PS/2 controller init**: flush output buffer, configure command byte, enable IRQ1/IRQ12
- **Keyboard driver**: scan-code set 2 decode, modifier-key state (Shift, Ctrl, Alt, Caps Lock)
- **Keyboard event queue**: lock-free ring buffer fed by the IRQ handler, consumed by the terminal layer
- **Serial input** *(optional)*: enable RX interrupt on UART so serial can mirror the keyboard path

---

## Phase 4 — Terminal / TTY Layer
*Turns raw key events into an interactive line-editing experience.*

- **Canonical line discipline**: echo typed characters, handle Backspace / Ctrl+U (kill line) / Ctrl+C
- **`read_line()` blocking call**: blocks the caller until `\n` or EOF, returns owned `String`
- **Cursor rendering**: blinking block cursor at current column; erase/redraw on move
- **ANSI escape sequences**: CSI codes for cursor movement, color, clear-screen in the framebuffer writer
- **Scroll improvement**: ensure smooth scroll in framebuffer writer for long output

---

## Phase 5 — Kernel Task Foundation
*A shell that can run sub-commands needs at least minimal cooperative tasking.*

- **Task / thread struct**: saved register file, kernel stack pointer, state (runnable / blocked / dead)
- **Context switch**: `switch_to(next: &Task)` — save callee-saved registers, swap RSP
- **Simple scheduler**: round-robin run-queue; `yield()` and `block()`/`wake()` primitives
- **Synchronization primitives**: `Mutex`, `Semaphore`, `WaitQueue` built on the scheduler

---

## Phase 6 — Shell
*The deliverable of all prior phases.*

- **Command-line parser**: tokenizer (quoted strings, whitespace splitting), argument vector
- **Built-in commands**: `help`, `echo`, `clear`, `meminfo`, `halt`, `uptime`, `lspci` stub
- **Command dispatch**: registry of name → handler fn; extensible for future external commands
- **History**: circular buffer of past lines; up/down arrow navigation via ANSI sequences
- **Tab completion** *(stretch)*: complete against the built-in command registry

---

## Phase 7 — Filesystem & Executable Loading
*Required only if the shell should run programs beyond built-ins.*

- **VirtIO block driver** (QEMU) or **ATA PIO driver** (bare metal): sector read/write
- **GPT partition table parser**
- **FAT32 or ext2 read-only driver**: directory listing, file open/read
- **ELF64 loader**: parse PT_LOAD segments, allocate pages, set entry point

---

## Phase 8 — User Mode & System Calls
*Required only if shell commands run in ring 3.*

- **User-mode page tables**: separate address space per process; kernel mapped high
- **SYSCALL/SYSRET entry point**: `MSR_LSTAR`, stack switch, argument convention
- **Core syscalls**: `read`, `write`, `exit`, `fork`/`exec` stubs, `mmap`
- **User-mode stack setup**: initial stack with `argv`/`envp`, ABI-compliant entry

---

## Notes

**Minimum viable shell** requires Phases 1–4 (plus Phase 5 if commands need to run concurrently) — all without touching filesystems or user mode.

**Phases 7–8** are what turn the shell into a general-purpose OS entry point.
