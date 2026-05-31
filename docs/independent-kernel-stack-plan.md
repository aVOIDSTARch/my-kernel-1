# Independent Kernel Stack Plan

**Goal:** Complete independence from Limine-owned memory before `boot.release()`
is called, so that every `BootloaderReclaimable` page — including the 756 KiB
region `0x1ff23000..0x1ffe0000` currently containing the live boot stack — can
be returned to the buddy without restriction.

**Current state (commit `06afb97`):** The kernel boots cleanly. Two
`BootloaderReclaimable` regions are permanently skipped in `release()`: the
sub-1 MiB BDA region (not viable regardless) and the boot stack region (756 KiB
of real memory lost). The skip is implemented by reading RSP and comparing
against region boundaries. Once the kernel runs on its own stack, the SP guard
can be removed and the full region reclaimed.

---

## Precise Definition of "Independence"

The kernel is independent from Limine when, at the moment `release()` is called,
no register, stack frame, or pointer anywhere in the live kernel state points
into a `BootloaderReclaimable` physical page. Concretely:

| Resource | Current owner | Target owner |
|---|---|---|
| Stack (`RSP`) | Limine (in `0x1ff23000..0x1ffe0000`) | Buddy-allocated frame from `Usable` memory |
| Page tables (CR3) | Limine (frames in `BootloaderReclaimable`) | Kernel-built PML4 from buddy frames |
| IST stacks (TSS) | Kernel `.bss` static arrays | Buddy-allocated frames, re-pointed in TSS |
| GDT | `lazy_static` in kernel image | Already kernel-owned — no change needed |
| IDT | `lazy_static` in kernel image | Already kernel-owned — no change needed |

The IST stacks in `gdt.rs` (`DOUBLE_FAULT_STACK`, `NMI_STACK`,
`MACHINE_CHECK_STACK`) live in the kernel's `.bss` section, which is
`ExecutableAndModules` — not reclaimable. They are already independent.
The plan does not touch them.

The two real tasks are: **(1) new kernel stack**, **(2) new kernel PML4**.
They must be done in this order because switching the stack is simpler and can
be validated in isolation; building the PML4 depends on a stable stack.

---

## Phase 1 — Allocate and Switch to a Kernel Stack

### 1.1 What the new stack needs

- Allocated from the buddy (order-0 blocks = 4 KiB each; 4 pages = 16 KiB is
  the minimum, 8 pages = 32 KiB is comfortable for the current call depth).
- Physical address in a `Usable` region — the buddy at this point only contains
  `Usable` pages, so any successful allocation satisfies this.
- The virtual address is `HHDM + phys` — already mapped by Limine's HHDM.
  No new mapping is needed for the stack itself.
- A guard page immediately below the stack: unmap the page at
  `stack_virt_base - 0x1000` so a stack overflow faults with CR2 pointing
  at the guard rather than silently corrupting the adjacent allocation.

### 1.2 Where to implement it

Add `memory::stack` module (`src/memory/stack.rs`) with a single public
function:

```rust
/// Allocate `pages` pages from the buddy as a kernel stack.
///
/// Returns the virtual address of the **top** of the stack (highest address,
/// since x86 stacks grow downward) and the virtual address of the stack base
/// (for guard page installation).
///
/// # Safety
/// The buddy must be initialized and contain at least `pages + 1` free pages
/// (the extra page is the guard).
pub unsafe fn alloc_kernel_stack(pages: usize) -> (u64, u64) {
    // Allocate pages + 1: bottom page becomes the guard (unmapped).
    let order = pages.next_power_of_two().trailing_zeros() as usize;
    // For non-power-of-two page counts, allocate individual pages.
    // Simple approach: allocate pages one at a time and stack them.
    // Guard page: allocate one extra order-0 page at the base.
    let guard_virt = {
        let ptr = abalone::buddy::alloc_pages(0).expect("stack guard OOM");
        ptr as u64
    };
    let mut stack_top = guard_virt + 0x1000; // guard is below stack

    for _ in 0..pages - 1 {
        let page = abalone::buddy::alloc_pages(0).expect("stack page OOM");
        let _ = page; // buddy already gave contiguous pages if lucky;
                      // for simplicity allocate independently and use highest
    }
    // Cleaner: allocate pages+1 as a single order block (requires power-of-two).
    // See note below.
    (stack_top + (pages - 1) as u64 * 0x1000, guard_virt)
}
```

The cleaner approach is to allocate `(pages + 1)` rounded up to the next power
of two as a single buddy order block. For 8 stack pages + 1 guard = 9 pages,
round up to 16 (order-4). The bottom page is the guard; the top of the 15th
page is the initial SP. The extra frames at the top are wasted but the layout
is simple and correct.

Concrete implementation in `src/memory/stack.rs`:

```rust
// v0.0.1
use abalone::buddy::BUDDY;
use crate::memory::vmm;

/// Stack allocation result.
pub struct KernelStack {
    /// Virtual address of the top of the stack (initial RSP value).
    pub top:       u64,
    /// Virtual address of the guard page (the page below the stack base).
    /// This page is unmapped; writing to it produces a #PF.
    pub guard_virt: u64,
}

/// Allocate a kernel stack of at least `min_pages` pages with a guard page.
///
/// Allocates `1 << order` pages where `order` is the smallest value such that
/// `(1 << order) > min_pages`. The first page is left unmapped as a guard;
/// the remaining pages form the stack.
///
/// Returns the virtual stack top (initial RSP) and the guard page address.
///
/// # Safety
/// Buddy and VMM must both be initialized.
pub unsafe fn alloc_kernel_stack(min_pages: usize) -> KernelStack {
    // Round up to next power of two so the entire block is one buddy allocation.
    // min_pages=8 → order=4 (16 pages): 1 guard + 15 stack.
    let total_order = (min_pages + 1).next_power_of_two().trailing_zeros() as usize;
    let total_pages = 1usize << total_order;

    let base_virt = {
        let mut buddy = BUDDY.lock();
        buddy.alloc_pages(total_order).expect("kernel stack OOM") as u64
    };

    // Unmap the guard page (bottom of allocation).
    // Safety: base_virt is HHDM-mapped by Limine; unmapping removes the
    // Limine leaf PTE. Any write below the stack top will now #PF.
    unsafe {
        vmm::get().unmap(base_virt);
    }

    let guard_virt = base_virt;
    let stack_top  = base_virt + (total_pages as u64) * 0x1000;

    KernelStack { top: stack_top, guard_virt }
}
```

### 1.3 Switching RSP in `kernel_main`

The switch must happen in a naked or carefully controlled context. In Rust
`no_std` x86_64, the cleanest approach is a small inline `asm!` block:

```rust
// In kernel_main, immediately after vmm::init() and before boot.release():

let kstack = unsafe { memory::stack::alloc_kernel_stack(8) };
serial_println!("[kernel] stack: top={:#x} guard={:#x}",
    kstack.top, kstack.guard_virt);

// Switch RSP to the new stack. After this instruction, the call stack
// frames built since kernel_main entry are abandoned — this is safe
// because we do not return from kernel_main and we do not use any
// variables from the Limine stack afterward.
//
// The `call` instruction pushes a return address; `kernel_main_on_new_stack`
// is a -> ! function so it never returns. The pushed return address is
// never used.
unsafe {
    core::arch::asm!(
        "mov rsp, {new_sp}",
        "call {continue_fn}",
        new_sp   = in(reg) kstack.top,
        continue_fn = sym kernel_main_continue,
        options(noreturn),
    );
}
```

`kernel_main_continue` is a separate `extern "C" fn() -> !` that contains
all code from "release reclaimable pages" onward. The split keeps the unsafe
`asm!` block minimal. Note: `options(noreturn)` on the `asm!` block tells the
compiler this path does not return; the function diverges correctly.

Alternatively, use a helper in `src/memory/stack.rs`:

```rust
/// Switch to `new_sp` and call `entry()`. Does not return.
///
/// # Safety
/// `new_sp` must be a valid, writable stack top. `entry` must not return.
pub unsafe fn switch_stack(new_sp: u64, entry: fn() -> !) -> ! {
    unsafe {
        core::arch::asm!(
            "mov rsp, {sp}",
            "call {f}",
            sp = in(reg) new_sp,
            f  = in(reg) entry as u64,
            options(noreturn),
        );
    }
}
```

### 1.4 Removing the SP guard from `release()`

Once `release()` is called from a stack in `Usable` memory, the guard is no
longer needed. Remove this block from `limine_data.rs`:

```rust
// REMOVE after Phase 1:
if sp_phys_page >= base && sp_phys_page < end {
    continue;
}
```

The `current_sp` read and `sp_phys_page` computation can be removed too,
cleaning up the function to its original concise form (minus the sub-1 MiB
filter, which stays permanently).

### 1.5 Verification

After Phase 1, boot log should show:

```
[kernel] stack: top=0xffff800000xxxxxx guard=0xffff800000yyyyyy
[kernel] boot pages released          ← now releases 0x1ff23000..0x1ffe0000 fully
```

Confirm with a deliberate stack probe: write a recursive function with a large
stack frame in a `#[test_case]` to verify the guard page fires as a #PF rather
than a silent corruption.

---

## Phase 2 — Build and Install a Kernel-Owned PML4

### 2.1 Why this is needed

After Phase 1 the kernel stack is independent. But CR3 still points to Limine's
PML4, whose frame nodes live in `BootloaderReclaimable`. The sub-1 MiB filter
in `release()` prevents those nodes from being freed, but only by accident —
`0x1000..0x53000` is skipped because it's below the buddy base, not because the
kernel actively protects it. The Limine page-table frames at
`0x1f7dd000..0x1f7e5000` (32 KiB) and portions of `0x1f82d000..0x1ff22000` are
fed into the buddy by `release()`. Once in the buddy those frames can be
allocated and written — zeroing a live PT node — without the kernel knowing.

The kernel survives only because it never allocates from those specific frames
between `release()` and `hlt`. That is luck, not correctness.

### 2.2 What the new PML4 must map

At the time of CR3 switch, the CPU is executing kernel code and may be
handling interrupts. Every virtual address that could be touched between the
CR3 write and the first use of the new mappings must be present:

| Region | Why required |
|---|---|
| Kernel image (`0xffffffff80000000..`) | Currently executing code |
| HHDM (`0xffff800000000000..0xffff800100000000`) | All kernel data, buddy, TLSF, stacks |
| Framebuffer MMIO | Written by `println!` macros |
| IST stacks | Interrupt handlers may fire during switch |
| GDT / IDT | Already in kernel image, covered by kernel image mapping |

The HHDM must be mapped with 2 MiB pages (same as Limine) to avoid consuming
thousands of PT frames and to maintain equivalent TLB coverage. Limine mapped
512 GiB of HHDM in ~256 PD entries using 2 MiB pages; replicating this
requires 1 PML4 + 1 PDPT + 256 PDs = 258 frames. Attempting to map it with
4 KiB pages would require 1 + 1 + 256 + 131,072 = 131,330 frames (512 MiB of
page-table memory), which is not viable.

### 2.3 Implementation: `mantle::pml4`

Add `mantle/src/pml4.rs`:

```rust
// v0.0.1

use crate::{prot::Protection, table::PageTable};
use bitwise::paging::{pte_encode, pte_flags, vaddr_pt_index};
use abalone::buddy::BUDDY;

const PAGE_SIZE:   u64 = 0x1000;
const HUGE_2M:     u64 = 0x0020_0000;
const HUGE_1G:     u64 = 0x4000_0000;

/// Build a new PML4 covering all regions the kernel needs, then load it
/// into CR3, atomically replacing Limine's page tables.
///
/// After this returns, CR3 points to kernel-owned frames. All frames used
/// for the new page tables were sourced from the buddy (`Usable` memory).
///
/// # Safety
/// - Buddy and VMM must be initialized.
/// - Interrupts should be disabled for the duration (or IDT must be valid
///   under both old and new page tables, which it is since both map the
///   kernel image identically).
/// - Must not be called while running on a stack outside the HHDM.
pub unsafe fn install_kernel_pml4(
    hhdm:              u64,
    kernel_virt_start: u64,
    kernel_virt_end:   u64,
    kernel_phys_start: u64,
    phys_mem_size:     u64,   // upper bound of physical memory to HHDM-map
    fb_virt:           u64,
    fb_phys:           u64,
    fb_pages:          u64,
) {
    // Allocate root PML4 frame from buddy.
    let pml4_phys = alloc_zero_frame(hhdm);

    // 1. Map HHDM using 2 MiB huge pages.
    map_hhdm_2m(hhdm, pml4_phys, phys_mem_size);

    // 2. Map kernel image (4 KiB pages, correct protection per section).
    //    For now, map the entire image RWX — split by section in a later pass.
    map_range_4k(hhdm, pml4_phys,
        kernel_virt_start, kernel_phys_start,
        (kernel_virt_end - kernel_virt_start + PAGE_SIZE - 1) / PAGE_SIZE,
        Protection::KERNEL_RWX_BOOT);

    // 3. Map framebuffer MMIO (WC, already mapped under old tables).
    map_range_4k(hhdm, pml4_phys,
        fb_virt, fb_phys, fb_pages, Protection::MMIO_WC);

    // 4. Write CR3 — atomic from the CPU's perspective; TLB is flushed.
    unsafe {
        core::arch::asm!(
            "mov cr3, {pml4}",
            pml4 = in(reg) pml4_phys,
            options(nostack, preserves_flags),
        );
    }
    // Execution continues on the new page tables. Limine's PML4 frames are
    // now unreferenced and safe to free via release().
}
```

### 2.4 HHDM mapping with 2 MiB pages

```rust
fn map_hhdm_2m(hhdm: u64, pml4_phys: u64, phys_mem_size: u64) {
    // Number of 2 MiB pages needed to cover physical memory.
    let huge_pages = (phys_mem_size + HUGE_2M - 1) / HUGE_2M;

    for i in 0..huge_pages {
        let phys = i * HUGE_2M;
        let virt = hhdm + phys;

        // Walk/create PML4 → PDPT → PD (stopping at PD level for 2 MiB).
        let pml4  = pml4_phys;
        let pdpt  = descend_or_create_intermediate(hhdm, pml4, virt, 4);
        let pd    = descend_or_create_intermediate(hhdm, pdpt, virt, 3);

        // Install a 2 MiB PDE (PS bit = HUGE_PAGE).
        let idx = vaddr_pt_index(virt, 2) as usize;
        let pde = pte_encode(phys, HUGE_2M,
            pte_flags::PRESENT | pte_flags::WRITABLE
            | pte_flags::NO_EXECUTE | pte_flags::HUGE_PAGE | pte_flags::GLOBAL);
        let pd_ptr = (hhdm + pd) as *mut PageTable;
        unsafe { (*pd_ptr).write(idx, pde) };
    }
}
```

### 2.5 Determining `phys_mem_size`

Pass `boot.regions().iter().map(|r| r.end()).max().unwrap_or(0)` as
`phys_mem_size`. This is the highest physical address seen in the memory map,
rounded up to the next 2 MiB boundary inside `map_hhdm_2m`. This avoids
mapping physical address space that doesn't exist.

### 2.6 CR3 write and the interrupt window

The `mov cr3, rX` instruction flushes the entire TLB (except global entries,
but the new mappings have `GLOBAL` set for kernel pages, so this is
intentional). Between the write and the next instruction, the CPU may be
delivering a pending interrupt. The IDT's code and stack addresses must be
valid under the new page tables, which they are because:

- The IDT itself is in the kernel image — mapped by step 2.
- The interrupt handlers are in the kernel image — mapped by step 2.
- The IST stacks are in `.bss` (kernel image) — mapped by step 2.
- The `RSP0` in the TSS is not used for interrupts at ring 0 (already ring 0).
- The new kernel stack is in HHDM — mapped by step 1.

Disabling interrupts around the CR3 write is belt-and-suspenders and acceptable
for now:

```rust
x86_64::instructions::interrupts::disable();
install_kernel_pml4(...);
x86_64::instructions::interrupts::enable();
```

### 2.7 Updated `kernel_main` sequence

```rust
pub extern "C" fn kernel_main() -> ! {
    // Steps 1–4 unchanged (harvest, gdt, interrupts, heap, vmm).

    // Step 5: framebuffer mapping (under Limine's page tables, as before).

    // Step 6: NEW — allocate kernel stack.
    let kstack = unsafe { memory::stack::alloc_kernel_stack(8) };
    serial_println!("[kernel] kstack top={:#x}", kstack.top);

    // Step 7: NEW — build and install kernel PML4.
    // Interrupts are still disabled at this point (enabled in old step 6).
    unsafe {
        memory::pml4::install_kernel_pml4(
            boot.hhdm_offset,
            boot.kernel_virt_start,
            boot.kernel_virt_end,
            boot.kernel_phys_start,
            phys_mem_size,
            fb_virt, fb_phys, fb_pages,
        );
    }
    serial_println!("[kernel] pml4 ok");

    // Step 8: switch stack (now safe — PML4 maps the new stack via HHDM).
    unsafe {
        memory::stack::switch_stack(kstack.top, kernel_main_continue);
    }
}

extern "C" fn kernel_main_continue() -> ! {
    // Step 9: release — now truly safe, no Limine memory is live.
    unsafe { boot.release() };
    serial_println!("[kernel] boot pages released");

    // Steps 10+: enable interrupts, framebuffer println, hlt loop.
}
```

Note: `boot` must be passed to `kernel_main_continue`. The cleanest approach
is a `static mut BOOT_DATA: Option<LimineData>` set before the stack switch,
consumed immediately in `kernel_main_continue`. Alternatively, restructure
so `release()` is called from a closure captured in the new stack frame — but
`switch_stack` must be `fn() -> !`, not a closure, for the `asm!` call to work
safely. The static is the pragmatic choice.

---

## Phase 3 — Remove Workarounds and Reclaim Lost Memory

Once Phases 1 and 2 are complete:

1. **Remove the SP guard from `release()`** (per Phase 1.4). The
   `0x1ff23000..0x1ffe0000` region (756 KiB) is now freely reclaimable.

2. **Consider removing the sub-1 MiB filter.** The region `0x1000..0x53000`
   starts below the buddy base (`0xffff800000053000`). This is a constraint of
   `abalone::buddy` — it requires `add_region` calls with virtual addresses ≥
   `base`. Options:
   - Leave the filter in place (328 KiB permanently lost, acceptable).
   - Extend `BuddyAllocator::add_region` to accept regions below `base` by
     lowering `base` (requires a `set_base` call before any `add_region` but
     this region is added after initial seeding — needs `abalone` changes).
   - This is an `abalone` issue, not a `limine_data` issue. File it there.

3. **Update `release()` doc comment** to accurately reflect that the safety
   invariant is now: "no register or stack frame points into a
   BootloaderReclaimable page" — which is enforced structurally rather than by
   the SP runtime check.

---

## Summary of Files Changed

| File | Change |
|---|---|
| `src/memory/stack.rs` | New — `alloc_kernel_stack`, `switch_stack` |
| `mantle/src/pml4.rs` | New — `install_kernel_pml4`, `map_hhdm_2m`, `map_range_4k` |
| `mantle/src/lib.rs` | Add `pub mod pml4` |
| `src/memory/mod.rs` | Add `pub mod stack`, `pub mod pml4` (re-export) |
| `src/main.rs` | Restructure into `kernel_main` + `kernel_main_continue`; add steps 6–8 |
| `src/limine_data.rs` | Remove SP guard; update doc comment on `release()` |
| `abalone/src/buddy.rs` | (Phase 3 optional) lower-base extension |

---

## Risks and Mitigations

**CR3 switch with interrupts disabled:** Interrupts are already disabled at
entry (`kernel_main` comment: "Interrupts DISABLED"). They are enabled only
after `kernel_main_continue` returns from `release()`. The window with
interrupts disabled spans only the PML4 installation and stack switch — a few
dozen instructions. This is safe.

**HHDM 2 MiB mapping with wrong `phys_mem_size`:** If `phys_mem_size` is
underestimated, some HHDM addresses become unmapped. The buddy uses HHDM
addresses for all its pages. Any buddy allocation in the unmapped range causes
a #PF. Mitigation: use `max(region.end())` over all memory map entries,
including `Reserved` regions, to get the true physical address space extent.

**Buddy lock held across stack switch:** `switch_stack` must not be called
while holding the buddy lock. It is not — the lock is released before
`alloc_kernel_stack` returns. Verify no `BUDDY.lock()` guard is live at the
call site.

**`kernel_main_continue` and `boot` lifetime:** `LimineData` is currently a
local variable in `kernel_main`. It must survive until `kernel_main_continue`
calls `release()`. Use a `static mut Option<LimineData>` initialized before
the stack switch and consumed (taken, not borrowed) in `kernel_main_continue`.
Rust's ownership rules enforce that it is consumed exactly once.

---

## References

- Intel SDM Vol. 3A §4.10.4 — CR3 write and TLB invalidation behavior
- Intel SDM Vol. 3A §4.5 — 2 MiB page (PS bit) in PDE
- OSDev Wiki "Context Switching": https://wiki.osdev.org/Context_Switching
- OSDev Wiki "Page Tables": https://wiki.osdev.org/Page_Tables
- Limine Protocol spec — BootloaderReclaimable guarantee:
  https://github.com/limine-bootloader/limine/blob/stable/PROTOCOL.md
