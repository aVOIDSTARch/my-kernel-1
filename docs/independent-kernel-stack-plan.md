# Independent Kernel Stack Plan

**Goal:** Complete independence from Limine-owned memory, so that every
`BootloaderReclaimable` page can be returned to the buddy without restriction,
and CR3 points to kernel-owned page table frames.

**Current state (as of commit `61341a6`):**

| Phase | Status |
|---|---|
| Phase 1 — new kernel stack | **COMPLETE** |
| Phase 2 — new kernel PML4 | **Stub only** — `mantle/src/pml4.rs` exists but helpers unimplemented; not wired to `lib.rs` or called from `main.rs` |
| Phase 3 — remove workarounds | Not started |

**Outstanding correctness gap:** `boot.release()` is called before `alloc_kernel_stack`,
so Limine's PT frames enter the buddy before CR3 is updated. If a subsequent buddy
allocation (e.g., the PT frame allocated inside `unmap()` for the guard page) happens
to land on a former Limine PT frame, the live page tables are silently corrupted.
Installing the kernel PML4 before `boot.release()` closes this gap. Phase 2 is the
immediate priority.

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

## Phase 1 — Allocate and Switch to a Kernel Stack [COMPLETE]

### What was implemented

**Implementation in `src/memory/stack.rs` and `src/post_stack_state.rs`.**

The actual approach differs from the original plan in one key respect: `boot.release()`
is called *before* the stack switch (with the SP guard still active in `release()`),
not after. The boot stack region is captured into `PostStackState` before the switch
and manually added to the buddy via `buddy.add_region()` in `kernel_main_continue`
after the RSP is already on a `Usable` frame. This avoids threading `LimineData`
through a `static mut`.

The guard page uses `vmm::get().unmap()` to zero the leaf PTE in the current (Limine)
page tables. This is safe when done before the PML4 switch, because Limine's tables
are still live and mapped at this point.

**SP guard:** The SP guard in `limine_data.rs::release()` remains. It is still
needed because `release()` is called on the Limine stack. It must be removed (or
`release()` must be called after the PML4 install) once Phase 2 is complete.

**Boot log now shows:**

```
[kernel] boot pages released (boot stack region deferred)
[kernel] stack: top=0xffff800000xxxxxx guard=0xffff800000yyyyyy
...
[kernel] boot stack pages released: base=0xffff8000xxxxxxxx pages=NNN
```

---

## Phase 2 — Build and Install a Kernel-Owned PML4 [NEXT]

### 2.1 Why this is needed

CR3 still points to Limine's PML4, whose frame nodes live in `BootloaderReclaimable`.
`boot.release()` feeds those frames into the buddy before the PML4 is switched.
Once in the buddy, a subsequent allocation (e.g., the guard-page `unmap()` allocating
an intermediate PT frame) could land on a former Limine PT node and corrupt it while
the CPU is still walking those same tables on every memory access.

The current boot sequence avoids this by luck: the frames happen not to be re-allocated
before the guard-page `unmap()` call, and `unmap()` itself does not allocate
(it only zeroes a leaf PTE). But this is not guaranteed and will break under heap
pressure or if the boot sequence changes.

Installing the kernel PML4 **before** `boot.release()` makes correctness structural.

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

**`mantle/src/pml4.rs` already exists** (v0.0.2) and contains the
`install_kernel_pml4` entry point. It is **not yet complete**: the three helper
functions it calls — `alloc_zero_frame`, `map_hhdm_2m`, and `map_range_4k` — are
missing. The module is also not declared in `mantle/src/lib.rs`.

**Tasks remaining in `mantle/src/pml4.rs`:**

Add the following helpers (model them on `walker.rs::PageTableWalker` but operate
on a caller-supplied `pml4_phys` rather than reading CR3):

```rust
/// Allocate one order-0 frame from the buddy; return its physical address.
/// Panics on OOM — called only during early init when OOM is unrecoverable.
fn alloc_zero_frame(hhdm: u64) -> u64 {
    let virt = abalone::buddy::alloc_pages(0)
        .expect("pml4: buddy OOM during page table build") as u64;
    // Zero the frame so all PTEs start as Not Present.
    unsafe { core::ptr::write_bytes(virt as *mut u8, 0, 0x1000) };
    virt - hhdm
}

/// Resolve or create the intermediate page table at `level` for `vaddr`.
/// Returns the physical address of the next-level table. Panics on OOM.
unsafe fn descend_or_create(hhdm: u64, table_phys: u64, vaddr: u64, level: u32) -> u64 {
    let table = (hhdm + table_phys) as *mut PageTable;
    let idx   = vaddr_pt_index(vaddr, level) as usize;
    let entry = unsafe { (*table).read(idx) };
    if pte_is_present(entry) {
        pte_phys_addr(entry, PAGE_SIZE)
    } else {
        let child_phys = alloc_zero_frame(hhdm);
        let e = pte_encode(child_phys, PAGE_SIZE, pte_flags::PRESENT | pte_flags::WRITABLE);
        unsafe { (*table).write(idx, e) };
        child_phys
    }
}

/// Map `phys_mem_size` bytes of physical memory as HHDM using 2 MiB huge pages.
fn map_hhdm_2m(hhdm: u64, pml4_phys: u64, phys_mem_size: u64) {
    let huge_pages = (phys_mem_size + HUGE_2M - 1) / HUGE_2M;
    for i in 0..huge_pages {
        let phys = i * HUGE_2M;
        let virt = hhdm + phys;
        let pdpt_phys = unsafe { descend_or_create(hhdm, pml4_phys, virt, 4) };
        let pd_phys   = unsafe { descend_or_create(hhdm, pdpt_phys, virt, 3) };
        let pd = (hhdm + pd_phys) as *mut PageTable;
        let idx = vaddr_pt_index(virt, 2) as usize;
        let pde = pte_encode(
            phys, HUGE_2M,
            pte_flags::PRESENT | pte_flags::WRITABLE
                | pte_flags::NO_EXECUTE | pte_flags::HUGE_PAGE | pte_flags::GLOBAL,
        );
        unsafe { (*pd).write(idx, pde) };
    }
}

/// Map `pages` contiguous 4 KiB pages: virt_start..+pages*4K -> phys_start..+pages*4K.
fn map_range_4k(
    hhdm:       u64,
    pml4_phys:  u64,
    virt_start: u64,
    phys_start: u64,
    pages:      u64,
    prot:       Protection,
) {
    for i in 0..pages {
        let virt = virt_start + i * PAGE_SIZE;
        let phys = phys_start + i * PAGE_SIZE;
        let pdpt_phys = unsafe { descend_or_create(hhdm, pml4_phys, virt, 4) };
        let pd_phys   = unsafe { descend_or_create(hhdm, pdpt_phys, virt, 3) };
        let pt_phys   = unsafe { descend_or_create(hhdm, pd_phys,   virt, 2) };
        let pt = (hhdm + pt_phys) as *mut PageTable;
        let idx = vaddr_pt_index(virt, 1) as usize;
        let leaf = pte_encode(phys, PAGE_SIZE, pte_flags::PRESENT | prot.bits());
        unsafe { (*pt).write(idx, leaf) };
    }
}
```

Also add `use bitwise::paging::pte_is_present;` and `use bitwise::paging::pte_phys_addr;`
to the imports (they are used by `descend_or_create`).

**`mantle/src/lib.rs`:** Add `pub mod pml4;`.

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

The call order must be: **PML4 install → `boot.release()` → stack switch**.
Installing before release ensures CR3 never points to frames that the buddy
could re-allocate. The current code has `release()` before stack allocation;
that must be corrected.

```rust
// Step 5.5 (existing): capture boot stack region before release.

// Step 5.6: NEW — install kernel PML4.
// Interrupts are disabled. IDT, IST stacks, and kernel image are all mapped
// in the new PML4, so any interrupt that fires during the CR3 write is safe.
let phys_mem_size = boot.regions().iter().map(|r| r.end()).max().unwrap_or(0);
unsafe {
    let (fb_virt, fb_phys, fb_pages) = boot.framebuffer
        .map(|fb| (fb.virt_addr, fb.phys_addr, (fb.byte_size + 0xFFF) / 0x1000))
        .unwrap_or((0, 0, 0));
    mantle::pml4::install_kernel_pml4(
        boot.hhdm_offset,
        boot.kernel_virt_start,
        boot.kernel_virt_end,
        boot.kernel_phys_start,
        phys_mem_size,
        fb_virt, fb_phys, fb_pages,
    );
}
serial_println!("[kernel] pml4 ok");

// Step 6 (existing, now safe): release reclaimable pages.
// CR3 no longer points to Limine's PT frames, so they are safe to free.
unsafe { boot.release() };

// Step 7 (existing): allocate new kernel stack.
let kstack = unsafe { memory::stack::alloc_kernel_stack(8) };

// Step 8 (existing): store PostStackState and switch.
post_stack_state::store(PostStackState { rsdp_phys, boot_stack_region });
unsafe { memory::stack::switch_stack(kstack.top, kernel_main_continue) };
```

After this change, the SP guard in `release()` is still needed (RSP is still on
the Limine stack when `release()` is called). Phase 3 removes it.

---

## Phase 3 — Remove Workarounds and Reclaim Lost Memory

Once Phases 1 and 2 are complete:

1. **Remove the SP guard from `release()`** in `limine_data.rs`. The block:

   ```rust
   if sp_phys_page >= base && sp_phys_page < end { continue; }
   ```

   and the two lines above it (`current_sp` read + `sp_phys_page` computation)
   become dead code once the kernel PML4 is installed before `release()` is called.
   The boot stack region is still captured explicitly via `boot_stack_region` and
   added to the buddy in `kernel_main_continue`, so reclamation is unaffected.

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
   invariant is now: "kernel PML4 must be installed before this is called" —
   which is enforced structurally rather than by the SP runtime check.

---

## Summary of Files Changed

| File | Phase | Status | Change |
|---|---|---|---|
| `src/memory/stack.rs` | 1 | Done | `alloc_kernel_stack`, `switch_stack` |
| `src/post_stack_state.rs` | 1 | Done | `PostStackState` cell for cross-switch data |
| `src/main.rs` | 1 | Done | `kernel_main` + `kernel_main_continue` split; boot stack capture |
| `mantle/src/pml4.rs` | 2 | Stub | Add `alloc_zero_frame`, `descend_or_create`, `map_hhdm_2m`, `map_range_4k` |
| `mantle/src/lib.rs` | 2 | Todo | Add `pub mod pml4` |
| `src/main.rs` | 2 | Todo | Insert PML4 install step before `boot.release()` |
| `src/limine_data.rs` | 3 | Todo | Remove SP guard; update `release()` doc comment |
| `abalone/src/buddy.rs` | 3 | Optional | Lower-base extension for sub-1 MiB region |

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
