# Kernel State — Snapshot 1

## Overview

A bare-metal x86_64 kernel written in Rust (`no_std`, edition 2024, nightly toolchain). Boots via the [Limine](https://github.com/limine-bootloader/limine) bootloader protocol. The kernel reaches a stable halt loop with interrupts enabled after initializing the GDT, IDT, PIC, and physical memory manager. No output subsystem exists yet; `panic!` halts silently.

---

## Build & Target

| Item | Value |
|---|---|
| Target spec | `x86_64-crusty_os.json` (custom) |
| Kernel virtual base | `0xFFFFFFFF80000000` |
| Linker | `rust-lld` via `lld.lld` flavor |
| Linker script | `kernel.ld` (passed via `-Tkernel.ld`) |
| SSE / MMX | Disabled (`-mmx,-sse,+soft-float`) |
| Red zone | Disabled |
| `build-std` | `core`, `compiler_builtins` (with mem functions) |
| Boot ISO | `make iso` → `my-kernel.iso` via xorriso + Limine |
| QEMU run | `make run` → `qemu-system-x86_64` with `-cdrom`, serial stdio |

The Makefile also has a `run-kvm` target for KVM-accelerated testing.

---

## Boot Sequence (`src/main.rs`)

```
Limine → kernel_main()
  1. Assert Limine base revision is supported
  2. Consume Limine responses (HHDM, memory map, kernel physical address)
  3. gdt::init()       — load GDT + TSS, set segment registers
  4. interrupts::init() — load IDT, initialize 8259 PIC
  5. pmm::init()       — populate free-page bitmap from USABLE entries
  6. pmm::reclaim_bootloader_memory() — add BOOTLOADER_RECLAIMABLE pages
  7. sti               — enable interrupts
  8. hlt loop          — kernel idles
```

Limine responses consumed at boot: `HhdmRequest`, `MemmapRequest`, `ExecutableAddressRequest`, `BaseRevision`.

---

## GDT / TSS (`src/gdt.rs`)

- **Segments**: kernel code (64-bit), kernel data, TSS.
- **Registers set**: CS, DS, SS (null), TR.
- **IST stacks** (20 KiB each, statically allocated):

| IST index | Use |
|---|---|
| 0 (`DOUBLE_FAULT_IST_INDEX`) | Double fault (`#DF`) |
| 1 (`NMI_IST_INDEX`) | Non-maskable interrupt |
| 2 (`MACHINE_CHECK_IST_INDEX`) | Machine check (`#MC`) |

---

## Interrupt Descriptor Table (`src/interrupts/exceptions.rs`)

All 20 standard x86_64 CPU exceptions are wired. Three use IST entries (NMI, #DF, #MC). Seven hardware IRQ slots are populated at PIC offsets 0x20–0x2F.

### Exception handlers (all in `src/interrupts/handlers.rs`)

| Vector | Exception | Action |
|---|---|---|
| 0x00 | #DE Divide Error | `panic!` |
| 0x01 | #DB Debug | return (trap) |
| 0x02 | NMI | `panic!` |
| 0x03 | #BP Breakpoint | return (trap) |
| 0x04 | #OF Overflow | return (trap) |
| 0x05 | #BR Bound Range | `panic!` |
| 0x06 | #UD Invalid Opcode | `panic!` |
| 0x07 | #NM Device Not Available | `panic!` |
| 0x08 | #DF Double Fault | `panic!` (IST 0) |
| 0x0A | #TS Invalid TSS | `panic!` |
| 0x0B | #NP Segment Not Present | `panic!` |
| 0x0C | #SS Stack Segment | `panic!` |
| 0x0D | #GP General Protection | `panic!` |
| 0x0E | #PF Page Fault | `panic!` with CR2 address |
| 0x10 | #MF x87 FP | `panic!` |
| 0x11 | #AC Alignment Check | `panic!` |
| 0x12 | #MC Machine Check | `panic!` (IST 2, diverging) |
| 0x13 | #XM SIMD FP | `panic!` |
| 0x14 | #VE Virtualization | `panic!` |

### Hardware IRQ handlers

| Vector | IRQ | Action |
|---|---|---|
| 0x20 | PIT Timer (IRQ0) | `dispatch::dispatch(0)`, EOI |
| 0x21 | PS/2 Keyboard (IRQ1) | decode scancode via `PS2Keyboard`, EOI |
| 0x27 | PIC Master Spurious (IRQ7) | ISR check, EOI only if genuine |
| 0x28 | RTC (IRQ8) | dismiss RTC (read register C), `dispatch::dispatch(8)`, EOI |
| 0x2C | PS/2 Mouse (IRQ12) | read port 0x60, `dispatch::dispatch(12)`, EOI |
| 0x2E | ATA Primary (IRQ14) | `dispatch::dispatch(14)`, EOI |
| 0x2F | ATA Secondary (IRQ15) | ISR check, master-only EOI if spurious |

Every handler calls `stats::record(vector)` on entry.

---

## 8259 PIC (`src/interrupts/pic.rs`)

- Master PIC at I/O ports 0x20/0x21, offset **0x20** (vectors 0x20–0x27).
- Slave PIC at I/O ports 0xA0/0xA1, offset **0x28** (vectors 0x28–0x2F).
- Initialized by `pic::init()` via `pic8259` crate.
- Spurious IRQ detection reads the In-Service Register (ISR) via OCW3 before sending EOI.
- Helper API (not yet called from `main`): `mask_irq`, `unmask_irq`, `read_imr`.

---

## IRQ Dispatch (`src/interrupts/dispatch.rs`)

A lock-free 16-slot `AtomicPtr` table maps IRQ lines 0–15 to `fn()` callbacks. `register`/`unregister` use compare-exchange for safe concurrent access. `dispatch(irq)` is called from timer, RTC, mouse, and ATA handlers. No handlers are registered yet.

---

## Keyboard Decoder (`src/interrupts/handlers.rs`)

Uses `pc-keyboard 0.9` crate: `PS2Keyboard<Us104Key, ScancodeSet1>` protected by a `spin::Mutex`. Decodes raw PS/2 scancodes from port 0x60 into `KeyEvent` and then `DecodedKey`. Decoded keys are currently discarded — no input subsystem exists yet.

---

## Interrupt Statistics (`src/interrupts/stats.rs`)

256-slot `[AtomicU64; 256]` array. `record(vector)` does a relaxed fetch-add. `count(vector)` returns the current total. `reset_all()` is unsafe (for test harnesses). Not yet exposed to any query interface.

---

## Physical Memory Manager (`src/memory/pmm.rs`)

A bitmap allocator covering up to **64 GiB** of physical address space (4 KiB pages).

- **Bitmap**: 2 MiB static `[u8; BITMAP_SIZE]` behind a `spin::Mutex`. All bits start set (reserved).
- **`init(entries, kernel_phys_start, kernel_phys_end, hhdm_offset)`**: iterates USABLE memory map entries, marks page-aligned ranges free, excludes the kernel image.
- **`reclaim_bootloader_memory(entries)`**: called after Limine responses are no longer needed; frees BOOTLOADER_RECLAIMABLE ranges.
- **`alloc_page() -> Option<u64>`**: linear scan, returns physical address of first free 4 KiB page.
- **`free_page(phys_addr)`**: marks page free (unsafe).
- **`phys_to_virt(phys) -> u64`**: adds HHDM offset.
- `HHDM_OFFSET` and `TOTAL_FREE_PAGES` are mutable statics (accessed only during init and from `free_page`).

`alloc_page`/`free_page`/`phys_to_virt` are implemented but not yet called.

---

## APIC Infrastructure (`src/interrupts/apic.rs`)

Written but **not initialized or called** from the boot sequence. Exists as future infrastructure.

- `apic_supported()`: CPUID.1 EDX bit 9 check.
- `set_mapped_vaddr(vaddr)`: stores MMIO virtual address in an atomic.
- `LocalApic`: MMIO register read/write via volatile, `init(spurious_vector)`, `end_of_interrupt()`, `send_ipi(dest, vector)`.
- `disable_pic()`: masks both 8259 controllers by writing 0xFF to data ports.

---

## Vector Table (`src/interrupts/vectors.rs`)

Single source of truth for all interrupt vector numbers. No raw numeric literals exist in other files. Also defines an `InterruptVector` enum (unused by live code yet) and `APIC_SPURIOUS = 0xFF`.

---

## Not Yet Implemented

- **Output**: no serial, VGA, or framebuffer driver. `panic!` halts without printing.
- **Virtual memory**: no page table management or VMM.
- **Heap**: no global allocator; `alloc` crate not linked.
- **Scheduler**: no tasks or processes.
- **System calls**: no syscall/sysenter mechanism.
- **APIC**: infrastructure present, not initialized.
- **IRQ consumers**: `dispatch` table is empty; timer ticks are counted but not acted on.
- **Device drivers**: no block, network, or framebuffer drivers.
- **SMP**: single-core only.
