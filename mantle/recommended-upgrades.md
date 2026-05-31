# VMM Review & Recommended Upgrades

**Files reviewed:** `walker.rs` (v0.0.2), `table.rs` (v0.0.2), `prot.rs` (v0.0.2),
`heap.rs`, `main.rs`, `limine_data.rs`
**Architecture:** x86_64, Limine boot protocol, Rust `#![no_std]` + `alloc`

The walker is structurally sound for a v0 kernel: the HHDM translation model
is correct, `translate` handles 1 GiB and 2 MiB huge pages properly, `unmap`
exists (the previous inferred review assumed it didn't), and `invlpg` is issued
on every leaf modification. What follows is a ranked list of concrete defects
and missing capabilities, ordered by severity, with specific line references.

---

## Critical Defects

### C1. `Protection::MMIO` Sets PWT+PCD — Produces Write-Through, Not UC

**File:** `prot.rs` — `Protection::MMIO`
**Severity: Critical**

```rust
pub const MMIO: Self = Self(
    pte_flags::WRITABLE
        | pte_flags::NO_EXECUTE
        | pte_flags::CACHE_DISABLE   // PCD = 1
        | pte_flags::WRITE_THROUGH,  // PWT = 1
);
```

On x86_64 the caching type for a page is determined by the intersection of
the PWT/PCD bits in the PTE and the PAT MSR (IA32_PAT, MSR `0x277`). With
PWT=1 and PCD=1 the CPU selects PAT entry index 3 (PAT3). The BIOS/firmware
default encoding of PAT3 is **Write-Combining (WC)**, not Uncacheable (UC).

The Intel SDM default PAT layout (Vol. 3A §11.12.4, Table 11-10):

| Index | PWT | PCD | PAT | Default type |
|-------|-----|-----|-----|--------------|
| 0     | 0   | 0   | 0   | WB           |
| 1     | 1   | 0   | 0   | WT           |
| 2     | 0   | 1   | 0   | UC-          |
| 3     | 1   | 1   | 0   | UC (strong)  |
| 4     | 0   | 0   | 1   | WB           |
| 5     | 1   | 0   | 1   | WT           |
| 6     | 0   | 1   | 1   | UC-          |
| 7     | 1   | 1   | 1   | UC (strong)  |

So PWT=1 + PCD=1 → PAT index 3 → **UC (strong)** under the Intel default.
That is actually the correct result for a strict uncacheable MMIO mapping.
The current code accidentally gets the right answer on Intel hardware with
default firmware.

However there are two live problems:

1. **AMD firmware does not guarantee this PAT layout.** AMD's BIOS/AGESA
   implementations have shipped with non-default PAT3 encodings. Without
   reading and validating IA32_PAT before the first MMIO mapping, the kernel
   silently produces the wrong caching type on affected hardware. For a
   framebuffer this means speculative write reordering and torn pixel writes.

2. **The framebuffer should be Write-Combining, not UC.** UC (strong)
   serializes every store. WC allows the write-combining buffers to coalesce
   pixel stores into 64-byte burst transactions before committing to the
   memory bus. For a 1920×1080×32bpp framebuffer the throughput difference is
   roughly 4–8×. The correct flag combination for WC under the default PAT is
   PWT=1, PCD=0, PAT=0 (index 1 — but index 1 is WT by default). To get WC
   the kernel must reprogram PAT MSR to place WC at a usable index.

**Fix:**

```rust
// In vmm init, after reading CR3:
pub fn init_pat() {
    // Standard layout: WB / WT / UC- / UC / WB / WT / UC- / WC
    // Index 7 = PWT|PCD|PAT = Write-Combining for framebuffer use.
    const PAT_VALUE: u64 =
        0x00 |           // PAT0: WB
        (0x04 << 8)  |   // PAT1: WT
        (0x05 << 16) |   // PAT2: UC-
        (0x06 << 24) |   // PAT3: UC (strong)
        (0x00 << 32) |   // PAT4: WB
        (0x04 << 40) |   // PAT5: WT
        (0x05 << 48) |   // PAT6: UC-
        (0x01 << 56);    // PAT7: WC  ← new
    unsafe { wrmsr(0x277, PAT_VALUE); }
}

// Then change Protection::MMIO to use PAT7 (PWT|PCD|PAT bit in PTE):
pub const MMIO_UC: Self = Self(
    pte_flags::WRITABLE | pte_flags::NO_EXECUTE
    | pte_flags::CACHE_DISABLE | pte_flags::WRITE_THROUGH  // PAT3 = UC
);
pub const MMIO_WC: Self = Self(
    pte_flags::WRITABLE | pte_flags::NO_EXECUTE
    | pte_flags::CACHE_DISABLE | pte_flags::WRITE_THROUGH | pte_flags::PAT
    // PAT7 = WC after kernel sets PAT MSR
);
```

The framebuffer call in `main.rs` should use `MMIO_WC`. Device register bars
that require strict ordering (e.g. PCIe config space) should use `MMIO_UC`.

---

### C2. `alloc_table_frame` Calls `abalone::buddy::alloc_pages(0)` Without Locking

**File:** `walker.rs` — `alloc_table_frame`
**Severity: Critical**

```rust
fn alloc_table_frame(&self) -> Option<*mut PageTable> {
    let virt = abalone::buddy::alloc_pages(0)? as u64;
    ...
}
```

`PageTableWalker` is accessed via `vmm::get()` which (from `main.rs`'s usage)
is behind a `spin::Mutex`. But `abalone::buddy::alloc_pages` presumably takes
the `BUDDY` global lock internally. If `alloc_table_frame` is called while the
buddy lock is already held by the same CPU — which cannot happen on the current
single-core kernel but will happen the moment SMP or re-entrant allocation is
introduced — this deadlocks.

More immediately: if the buddy's internal `alloc_pages(0)` function is not
`#[inline]` and returns a HHDM-virtual address (as the subtraction `frame_virt
as u64 - self.hhdm` implies it does), then the virtual-to-physical conversion
in `descend_or_create` is:

```rust
let frame_phys = frame_virt as u64 - self.hhdm;
```

This is correct only if `abalone::buddy::alloc_pages` returns a virtual
address in the HHDM window. If it ever returns a raw physical address (which
would be the natural return type for a physical frame allocator), the
subtraction produces a garbage physical address that gets written into the page
table, causing a page fault or triple fault on next access to any virtual
address routed through that intermediate table.

This implicit contract — that `alloc_pages` returns HHDM-virtual, not physical
— is not documented anywhere in `walker.rs` and is invisible to future
maintainers.

**Fix:**

```rust
// Document the contract explicitly, or better, make it unambiguous by type:
fn alloc_table_frame(&self) -> Option<*mut PageTable> {
    // abalone::buddy::alloc_pages returns a HHDM-virtual address.
    // Physical = virt - self.hhdm. This holds because all buddy pages are
    // mapped under the HHDM at init time.
    let virt = abalone::buddy::alloc_pages(0)? as u64;
    debug_assert!(virt >= self.hhdm, "buddy returned address below HHDM");
    debug_assert!((virt - self.hhdm) < MAX_PHYS_ADDR, "buddy returned implausible phys");
    let table = virt as *mut PageTable;
    unsafe { (*table).zero() };
    Some(table)
}
```

Long-term: introduce a `PhysFrame` newtype so the compiler catches the
virtual/physical confusion at the type level.

---

### C3. `map` Overwrites an Existing Mapping Without Returning the Old Frame

**File:** `walker.rs` — `map`
**Severity: Critical (memory leak becoming use-after-free)**

```rust
pub unsafe fn map(&self, vaddr: u64, phys: u64, prot: Protection) -> Option<()> {
    ...
    let leaf = pte_encode(phys, PAGE_SIZE, pte_flags::PRESENT | prot.bits());
    pt.write(idx, leaf);
    unsafe { bitwise::instructions::invlpg(vaddr) };
    Some(())
}
```

The doc comment says "If `vaddr` was already mapped, the existing leaf entry
is overwritten." The old physical frame is silently discarded. The caller has
no way to know the old frame needs to be freed, and the walker does not free
it. For page-table frames allocated during `descend_or_create` this is also
true: if an intermediate entry is being modified and a frame already existed,
`descend_or_create` returns the existing frame's physical address without
allocating a new one — that part is correct. But the leaf overwrite losing the
old frame is not.

In a kernel that has no user-space processes yet this only manifests as a
slow memory leak from remapping kernel MMIO regions. Once user-space exists
and pages are remapped for CoW or mprotect, this becomes a use-after-free: the
physical frame is leaked from the page table but still pointed to by the old
mapping's PTE if TLB state is involved, or accessible via the HHDM if the buddy
hands it out again.

**Fix:**

```rust
pub unsafe fn map(&self, vaddr: u64, phys: u64, prot: Protection)
    -> Result<Option<u64>, AllocError>
{
    // ... descend/create ...
    let idx = vaddr_pt_index(vaddr, 1) as usize;
    let old = pt.read(idx);
    let old_frame = if pte_is_present(old) {
        Some(pte_phys_addr(old, PAGE_SIZE))
    } else {
        None
    };
    let leaf = pte_encode(phys, PAGE_SIZE, pte_flags::PRESENT | prot.bits());
    pt.write(idx, leaf);
    unsafe { bitwise::instructions::invlpg(vaddr) };
    Ok(old_frame)  // caller decides whether to free it
}
```

---

## High-Severity Defects

### H1. `descend_or_create` Sets USER_ACCESSIBLE on No Intermediate Entry — But More Importantly, It May Not

**File:** `walker.rs` — `descend_or_create`
**Severity: High**

```rust
let new_entry = pte_encode(frame_phys, PAGE_SIZE,
    pte_flags::PRESENT | pte_flags::WRITABLE);
```

Intermediate table entries (PML4E, PDPTE, PDE) must have `USER_ACCESSIBLE`
set if any leaf beneath them maps a user-space page. They must not have
`NO_EXECUTE` set — that bit is reserved at intermediate levels on some
microarchitectures and causes a `#GP` on others. The `NO_EXECUTE` flag is
only meaningful in leaf PTEs.

The current code does not set `NO_EXECUTE` on intermediate entries, which is
correct. It does not set `USER_ACCESSIBLE`, which is correct for a kernel-
only mapping. But it is not enforced anywhere: a future caller adding a
user-space `Protection` variant and calling `map` will silently create leaf
PTEs with `USER_ACCESSIBLE` while all intermediate entries lack it, producing
a mapping that the hardware refuses to use — the page fault handler fires with
CR2 set correctly but the mapping apparently present.

**Fix:** Add `Protection::USER_RO` / `USER_RW` variants that signal to `map`
to propagate `USER_ACCESSIBLE` up through all intermediate entries it creates
or touches. The walker needs to know at walk time, not just at leaf write time,
whether the mapping is user-accessible.

---

### H2. Intermediate Table Entries Have No NX Control — and No GLOBAL Flag

**File:** `walker.rs` — `descend_or_create`, `map`
**Severity: High**

Kernel mappings should set the `GLOBAL` flag (bit 8) in leaf PTEs. When CR4.PGE
is set (enabled by default on x86_64, set by the BIOS long before Limine hands
off), global pages are not flushed from the TLB on CR3 reload. Without `GLOBAL`,
every context switch (once user processes exist) flushes all kernel TLB entries,
causing kernel entry/exit overhead proportional to the number of kernel pages
touched. On a kernel with a large HHDM this is severe.

`Protection` has no `GLOBAL` bit. The walker never sets it. The fix is
mechanical — add `pte_flags::GLOBAL` to every `Protection` constant that
represents a kernel mapping — but it must happen before user processes are
introduced, because retrofitting it after the fact requires flushing all kernel
TLB entries on all CPUs.

---

### H3. `table.rs` Uses `read_volatile` / `write_volatile` — Correct but Incomplete

**File:** `table.rs` — `read`, `write`
**Severity: High (correctness gap, not a bug today)**

```rust
pub fn read(&self, index: usize) -> u64 {
    unsafe { ptr::read_volatile(&self.entries[index]) }
}
pub fn write(&mut self, index: usize, value: u64) {
    unsafe { ptr::write_volatile(&mut self.entries[index], value) }
}
```

`read_volatile` / `write_volatile` prevent the *compiler* from reordering or
eliding the access. They do not prevent the *CPU* from reordering stores to
page-table memory relative to the `invlpg` or CR3 write that makes those
stores visible to the MMU's page-table walker.

On x86 the TSO memory model makes this safe in practice: stores are observed
in program order by the local CPU, and `invlpg` is a serialising instruction
with respect to subsequent memory accesses on the same CPU. So the current
code is correct on x86_64.

The problem is documentation: nothing in `table.rs` or `walker.rs` states this
assumption. When this code is ported to AArch64 (where the memory model
requires explicit `DSB ISHST` barriers before `TLBI` instructions) or reviewed
by someone unfamiliar with x86 TSO, the absence of barriers will look like a
bug. Add a comment at the top of `table.rs`:

```rust
// Memory ordering: x86_64's TSO model guarantees that stores to page-table
// memory are observed by the MMU walker in program order on the local CPU.
// read_volatile / write_volatile are sufficient to prevent compiler
// reordering. A port to weakly-ordered architectures (AArch64, RISC-V)
// requires DSB/FENCE instructions before TLB invalidation.
```

---

### H4. No Virtual Address Space Allocator

**File:** `main.rs`, `walker.rs`
**Severity: High**

`map_mmio(virt_start, phys_start, size)` takes an explicit virtual address.
The caller in `main.rs` passes `fb.virt_addr` directly from the Limine
response — a HHDM address that Limine chose. This works for the framebuffer.
It will not work for the next MMIO consumer (APIC at `0xFEE00000` is already
physical, needs a kernel VA; PCI BARs need VA assignment on every boot).

Without a VA allocator, every new MMIO mapping requires a human to pick a
non-conflicting address, verify it doesn't overlap the HHDM, kernel image, or
existing mappings, and hope that nobody else picks the same address. That is
not a sustainable model past two drivers.

**Fix:** Add a bump allocator over a reserved VA region to the VMM singleton.
See prior review §2.4 for a proposed virtual address layout. The allocator
itself is ~20 lines: an `AtomicU64` cursor advanced by `fetch_add`, clamped to
the region ceiling, aligned to 2 MiB.

---

### H5. `map` Returns `Option<()>` — OOM and Existing-Mapping Are Indistinguishable

**File:** `walker.rs` — `map`, `map_mmio`
**Severity: High**

`alloc_table_frame` returns `None` on buddy exhaustion. `descend_or_create`
propagates that `None` via `?`. `map` propagates it again. The caller receives
`None` and cannot distinguish "out of physical memory" from any other future
failure mode. `map_mmio` propagates `None` as well, and `main.rs` discards
the result entirely:

```rust
let _ = memory::vmm::get().map_mmio(...);
```

**Fix:**

```rust
#[derive(Debug)]
pub enum MapError {
    OutOfFrames,
    AlreadyMapped(u64),   // existing physical address
    Misaligned,
}
```

Return `Result<Option<u64>, MapError>` from `map` (the `Option<u64>` being the
old frame, per C3). Make `map_mmio` return `Result<(), MapError>`. Add a
`vmm_expect!` macro in `main.rs` that serial-prints the error and panics.

---

## Medium-Severity Issues

### M1. `PageTable::zero` Issues 512 Volatile Writes — Use `write_bytes`

**File:** `table.rs` — `zero`
**Severity: Medium (performance)**

```rust
pub fn zero(&mut self) {
    for i in 0..512 {
        self.write(i, 0);
    }
}
```

This is 512 separate volatile stores. The compiler cannot collapse them.
`core::ptr::write_bytes` emits a `rep stosq` (or equivalent) which the CPU
executes as a single micro-op sequence with hardware prefetch, typically 4–8×
faster for a 4 KiB page:

```rust
pub fn zero(&mut self) {
    // Safety: entries is [u64; 512], u64 is valid for all-zero bits,
    // and write_bytes is equivalent to memset on a properly aligned region.
    unsafe {
        core::ptr::write_bytes(self.entries.as_mut_ptr(), 0, 512);
    }
}
```

Page-table allocation happens on every `map` call that needs a new
intermediate node. At boot this is called dozens of times; at runtime it is
called whenever a new virtual region is mapped. The difference is measurable
in a boot-time flame graph.

---

### M2. `walk_existing` Borrows `table_at_phys` Without Lifetime Coupling to `self`

**File:** `walker.rs` — `walk_existing`, `table_at_phys`
**Severity: Medium (soundness risk)**

```rust
unsafe fn table_at_phys(&self, phys: u64) -> &mut PageTable {
    unsafe { &mut *(self.phys_to_virt(phys) as *mut PageTable) }
}
```

This returns `&mut PageTable` with lifetime tied to `&self` — but `&self` is
a shared reference, so the signature is:

```rust
fn table_at_phys<'a>(&'a self, phys: u64) -> &'a mut PageTable
```

This allows two `&mut PageTable` references to the same physical address to
exist simultaneously by calling `table_at_phys` twice with the same `phys`.
In `translate`, which calls `table_at_phys` four times in sequence, this is
fine because the references don't overlap in time. But `map` calls
`descend_or_create` (which calls `table_at_phys`) and then calls
`table_at_phys` again for the leaf table — if any two of the four levels happen
to alias (pathological but possible if a PML4 entry points to itself), you have
two live `&mut` to the same memory, which is undefined behaviour in Rust even
if the CPU handles it correctly.

The practical risk is low because normal page tables don't self-alias. The
theoretical risk is real because the `unsafe` contract on `table_at_phys` does
not exclude aliased physical addresses.

**Fix:** Return `*mut PageTable` from `table_at_phys` and use raw pointer
operations throughout `walker.rs`. This makes the aliasing explicit (raw
pointers opt out of Rust's aliasing model) and removes the false safety
implication of `&mut`. Alternatively, use a `NonNull<PageTable>` return type
with a documented "callers must not create two live references to the same
frame" requirement.

---

### M3. No Intermediate Table Reclamation in `unmap`

**File:** `walker.rs` — `unmap`
**Severity: Medium (memory leak)**

```rust
pub unsafe fn unmap(&self, vaddr: u64) {
    // ...
    pt.write(idx, 0);
    unsafe { bitwise::instructions::invlpg(vaddr) };
}
```

The doc comment acknowledges this: "Intermediate tables are not freed (they
may still serve other mappings)." This is correct behaviour — you cannot free
an intermediate table without checking whether all 512 of its entries are
zero. But the check is not performed, so even a fully empty intermediate
table is never reclaimed. Over time, in a kernel that maps and unmaps many
regions (driver load/unload, user process creation/destruction), the page-
table tree accumulates empty intermediate nodes that consume physical frames
permanently.

**Fix:** After zeroing the leaf entry, walk back up the tree and check each
intermediate table for emptiness. If all 512 entries are zero, zero that
intermediate entry in its parent and free the frame to the buddy. This is a
standard page-table compaction operation and is O(3) table scans regardless of
mapping density.

---

### M4. `Protection::KERNEL_RWX` Has No Deprecation Marker

**File:** `prot.rs`
**Severity: Medium (security)**

```rust
/// Read-write-execute. Use only during early boot; remove once code is loaded.
pub const KERNEL_RWX: Self = Self(pte_flags::WRITABLE);
```

The comment says "use only during early boot" but the constant is a public
`pub const` with no enforcement. Nothing prevents a driver or future subsystem
from mapping an arbitrary region RWX. W^X (write-XOR-execute) is a kernel
security baseline, not a preference.

**Fix:**

```rust
#[deprecated = "W^X violation. Use KERNEL_RX for code or KERNEL_RW for data. \
                Only acceptable during early boot JIT stubs with a known lifetime."]
pub const KERNEL_RWX: Self = Self(pte_flags::WRITABLE);
```

This produces a compiler warning on every use, forcing callers to acknowledge
the violation explicitly with `#[allow(deprecated)]`.

---

## Missing Capabilities

| Capability | Urgency | Blocking |
|---|---|---|
| PAT MSR init + `MMIO_WC` variant (→ C1) | Now | Framebuffer correctness |
| `MapError` enum, remove `Option<()>` (→ H5) | Now | Error handling discipline |
| `GLOBAL` flag in kernel `Protection` variants (→ H2) | Before user processes | TLB performance |
| `USER_ACCESSIBLE` propagation (→ H1) | Before user processes | User-space correctness |
| Virtual address range allocator (→ H4) | Before second MMIO driver | Address space sanity |
| Kernel-owned PML4 installed at boot | Before `boot.release()` | Memory safety |
| Intermediate table reclamation in `unmap` (→ M3) | Before driver lifecycle | Physical memory leak |
| Old-frame return from `map` (→ C3) | Before any remapping | Physical memory leak |
| TLB shootdown IPI stub | Before SMP | Correctness |
| 2 MiB huge page `map_huge` method | Before large region mapping | TLB performance |
| Guard pages below stacks | Before scheduler | Stack overflow detection |

---

## What Is Done Well

- **`translate` handles huge pages correctly.** The 1 GiB (PDPT) and 2 MiB
  (PD) huge page offsets are computed correctly with the right masks
  (`0x3FFF_FFFF` and `0x001F_FFFF`). This is easy to get wrong and is right.

- **`invlpg` is issued on every leaf write.** Both `map` and `unmap` call
  `bitwise::instructions::invlpg(vaddr)` after modifying the leaf entry.
  The flush happens after the write, which is the correct order (write first,
  then invalidate the stale TLB entry).

- **`walk_existing` refuses to follow huge pages.** In `unmap`, if an
  intermediate entry has the huge-page bit set, `walk_existing` returns `None`
  and the unmap silently no-ops. This prevents the walker from misinterpreting
  a 2 MiB PDE's physical address as a PT physical address, which would access
  an arbitrary memory location. This is the correct defensive behaviour.

- **`debug_assert!` on alignment in `map`.** Alignment checks in debug builds
  catch misaligned addresses before they corrupt the page table.

- **`table.rs` volatile access is consistent.** Every read and write goes
  through `read_volatile` / `write_volatile`. There are no accidental direct
  array accesses that would let the compiler cache a stale PTE value.

- **`Protection` constants cover the necessary kernel cases.** `KERNEL_RO`,
  `KERNEL_RW`, `KERNEL_RX` provide W^X-correct defaults. The `NO_EXECUTE` bit
  is set on data pages and absent on code pages, which is correct.

---

## References

- Intel SDM Vol. 3A §4.9: Paging and Memory Typing — PAT interaction
- Intel SDM Vol. 3A §11.12.4: PAT MSR defaults (Table 11-10)
- Intel SDM Vol. 3A §4.10.4: Invalidation of TLBs and Paging-Structure Caches
- OSDev Wiki "Page Attribute Table": https://wiki.osdev.org/Page_Attribute_Table
- OSDev Wiki "Global Pages": https://wiki.osdev.org/TLB#Global_Pages
