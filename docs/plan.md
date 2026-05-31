# Kernel -- Terminal Shell: Multiphase Roadmap

---

## Phase 1 -- Memory Management Foundation  [COMPLETE]

- [done] Virtual Memory Manager (VMM): PageTableWalker in mantle/ with
  map/unmap/translate/map_mmio; Protection::KERNEL_RO/RW/RX/MMIO_UC/MMIO_WC;
  PAT MSR programmed for write-combining at boot
- [done] Kernel Heap: BuddyAllocator (page-level) + TlsfAllocator (sub-page);
  wired as #[global_allocator] in src/memory/heap.rs
- [done] alloc crate support: extern crate alloc; Box, Vec, String, BTreeMap
  available kernel-wide
- [done] MMIO abstraction: map_mmio() with typed Protection variants;
  framebuffer mapped MMIO_WC; device BARs use MMIO_UC
- [done] BootloaderReclaimable release: LimineData::release() feeds reclaimed
  pages into buddy after VMM is up

---

## Phase 2 -- Interrupt & Timer Infrastructure  [COMPLETE]

- [done] GDT + TSS: GlobalDescriptorTable with kernel code/data/TSS; three IST
  stacks (double-fault, NMI, machine-check); loaded in gdt::init()
- [done] IDT + full exception set: all x86 exceptions wired with appropriate
  handlers (page fault panics with CR2 address, GPF panics with error code,
  double-fault via IST); spurious LAPIC vector 0xFF registered; interrupts
  enabled in kernel_main
- [done] PIC 8259 init: pic::init() programs both 8259s with offsets 0x20/0x28;
  mask_irq/unmask_irq/eoi helpers; spurious IRQ detection
- [done] IRQ dispatch table: dispatch::dispatch(irq) function-pointer registry;
  handlers for timer, keyboard, and other IRQ lines registered at boot
- [done] LAPIC enabled: apic_supported() checked via CPUID; LAPIC MMIO
  (0xFEE00000) mapped MMIO_UC and initialized at boot; init_lapic() in
  src/interrupts/apic.rs; disable_pic() ready for future I/O APIC migration
- [done] PIT timer at 1 kHz: pit programmed with divisor 1193 in src/timer.rs;
  timer_handler increments TICKS AtomicU64 on every IRQ0; uptime_ms() and
  sleep_ms() available kernel-wide
- [note] APIC timer and I/O APIC routing deferred: requires ACPI MADT parsing
  and is only needed for SMP or to replace the PIT; PIC stays for external IRQs

---

## Phase 3 -- Input Drivers  [PARTIAL]

- [partial] PS/2 keyboard: keyboard_handler() (PIC IRQ1) decodes scan codes via
  pc_keyboard crate (US104 layout, Set 1); no delivery mechanism yet --
  key events are decoded but not enqueued anywhere
- [todo] Keyboard event queue: lock-free ring buffer fed by IRQ handler
- [todo] PS/2 controller init: explicit flush/configure step before enabling IRQ
- [todo] Serial input RX interrupt: UART RX not yet enabled; serial is TX-only

---

## Phase 4 -- Terminal / TTY Layer  [OUTPUT DONE; INPUT NOT STARTED]

- [done] Framebuffer writer: FbWriter with 8x16 IBM PC glyph rendering, scroll,
  cursor tracking, interrupt-safe locking; init_from_info() in
  src/writers/framebuffer.rs
- [done] Serial writer: SERIAL1 (COM1) via uart_16550; serial_println! macro
  operational throughout boot
- [done] print!/println! macros: route through framebuffer when initialized,
  fall back to VGA text buffer
- [todo] Canonical line discipline: echo, Backspace, Ctrl+U, Ctrl+C
- [todo] read_line(): blocking call returning owned String
- [todo] Cursor rendering: blinking block cursor; erase/redraw on move
- [todo] ANSI escape sequences: CSI codes for cursor movement, color, clear

---

## Phase 5 -- Kernel Task Foundation  [NOT STARTED]

- [todo] Task/thread struct: saved register file, kernel stack pointer, state
- [todo] Context switch: switch_to(next) -- save callee-saved registers, swap RSP
- [todo] Simple scheduler: round-robin run-queue; yield()/block()/wake()
- [todo] Synchronization primitives: Mutex, Semaphore, WaitQueue

---

## Phase 6 -- Shell  [NOT STARTED]

- [todo] Command-line parser: tokenizer (quoted strings, whitespace splitting)
- [todo] Built-in commands: help, echo, clear, meminfo, halt, uptime, lspci stub
- [todo] Command dispatch: name-to-handler registry
- [todo] History: circular buffer; up/down arrow navigation
- [todo] Tab completion (stretch): complete against built-in command registry

---

## Phase 7 -- Filesystem & Executable Loading  [NOT STARTED]

- [todo] VirtIO block driver (QEMU) or ATA PIO driver (bare metal)
- [todo] GPT partition table parser
- [todo] FAT32 or ext2 read-only driver: directory listing, file open/read
- [todo] ELF64 loader: parse PT_LOAD segments, allocate pages, set entry point

---

## Phase 8 -- User Mode & System Calls  [NOT STARTED]

- [todo] User-mode page tables: separate address space per process
- [todo] SYSCALL/SYSRET entry: MSR_LSTAR, stack switch, argument convention
- [todo] Core syscalls: read, write, exit, fork/exec stubs, mmap
- [todo] User-mode stack: argv/envp, ABI-compliant entry

---

## Notes

Minimum viable shell requires Phases 1-4 complete plus Phase 5 if commands
need to run concurrently -- all without touching filesystems or user mode.

Immediate next steps from current state:
1. Complete Phase 3: keyboard event queue, deliver decoded keys to TTY layer
2. Phase 4 input: line discipline, read_line, cursor, ANSI sequences
3. Phase 5: cooperative tasking so the shell can block on input

Phases 7-8 are what turn the shell into a general-purpose OS entry point.
