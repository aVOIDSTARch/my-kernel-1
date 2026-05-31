# Abalone Crate Refactor Plan

## Executive Summary

The `abalone` crate works but has accumulated several structural problems:
the buddy allocator has a fundamental architectural flaw that breaks
multi-region operation, dead code that was never wired up ships as part of
the active crate, constants that belong to the kernel live here instead,
and the `TlsfAllocator` is tightly coupled to the buddy in a way that
prevents testing either in isolation. This document describes all issues
found and the specific changes required to fix them, in priority order.

---

## Issue 1 — Buddy allocator: single-base contiguous model (BLOCKING)

**Severity:** blocking — causes runtime panics whenever `release()` feeds a
reclaimable region whose physical address is below the first usable region.

**Root cause:** `add_region` computes a page index as:

```rust
let offset_pages = (virt_base - self.base) / PAGE_SIZE;
```

`self.base` is set to the first region ever registered. Any subsequent
region whose address falls below that base produces integer underflow and
triggers the assert on line 101:

```rust
assert!(virt_base >= self.base, "region base precedes allocator base");
```

The current memory map has `BootloaderReclaimable` starting at physical
`0x1000`, which maps to virtual `0xffff800000001000`, well below the first
`Usable` region at `0xffff800000053000`.

**Fix — establish base at global minimum before inserting any region:**

Change `add_region` to accept the possibility that an earlier base exists.
The correct fix is to establish `self.base` at the lowest address across
all regions before inserting any of them. This requires a two-pass
initialization:

```rust
// Pass 1: find the global minimum virtual address across all regions.
pub fn set_base(&mut self, virt_base: usize) {
    assert!(self.total_pages == 0, "set_base must be called before add_region");
    assert!(virt_base % PAGE_SIZE == 0);
    self.base = virt_base;
}

// Pass 2: add_region as before, but base is pre-established.
pub unsafe fn add_region(&mut self, virt_base: usize, page_count: usize) {
    assert!(self.base != 0, "call set_base before add_region");
    assert!(virt_base >= self.base, "region base precedes allocator base");
    // ... rest unchanged
}
```

The call site in `heap::init` becomes:

```rust
// Compute minimum virtual address across all usable + reclaimable regions.
let min_virt = regions
    .iter()
    .filter(|r| r.region_type.is_immediately_usable() || r.region_type.is_reclaimable())
    .map(|r| hhdm_offset + r.aligned_base())
    .min()
    .expect("no usable memory");

let mut buddy = BUDDY.lock();
buddy.set_base(min_virt as usize);
// then add_region calls as before
```

And `release()` in `limine_data.rs` no longer needs its skip guard — it can
feed every reclaimable region safely because the base is already set to the
minimum.

**Files changed:** `abalone/src/buddy.rs`, `src/memory/heap.rs`,
`src/limine_data.rs`

---

## Issue 2 — `TlsfAllocator::init` is tightly coupled to the buddy (STRUCTURAL)

**Current behaviour:** `init` calls `buddy::alloc_pages` directly via the
module-level free function, which goes through the global `BUDDY` mutex.
This means:

- TLSF cannot be tested without a live buddy.
- Changing the page source (e.g. using a different backing allocator) requires
  editing `tlsf.rs`.
- The `buddy` module is a hard dependency of `tlsf`, which inverts the
  intended layering (`buddy` should have no knowledge of `tlsf`).

**Fix — inject the initial pool as a raw pointer:**

```rust
impl TlsfAllocator {
    pub const fn new() -> Self { ... }

    /// Initialise TLSF from a caller-provided memory region.
    ///
    /// The caller is responsible for obtaining the memory (e.g. from the
    /// buddy) and passing a valid, writable, exclusively-owned pointer.
    ///
    /// # Safety
    /// `mem` must point to `size` bytes of writable memory that will not be
    /// aliased or freed for the lifetime of this allocator.
    pub unsafe fn init_from_ptr(&self, mem: *mut u8, size: usize) {
        unsafe { self.inner.lock().add_pool(mem, size); }
    }
}
```

The call site in `heap.rs`:

```rust
let mem = {
    let mut b = BUDDY.lock();
    b.alloc_pages(10).expect("TLSF init: buddy OOM")
};
unsafe { HEAP.init_from_ptr(mem, PAGE_SIZE << 10); }
```

Remove `use crate::buddy;` from `tlsf.rs`. The existing `init(buddy_order)`
method can be kept as a convenience wrapper but should be clearly marked as
the opinionated shorthand it is.

**Files changed:** `abalone/src/tlsf.rs`, `src/memory/heap.rs`

---

## Issue 3 — Dead code: `bump.rs` and `linked_list.rs` (HYGIENE)

**Current state:**
- `bump.rs` implements a bump allocator with `GlobalAlloc`. It is not wired
  to anything in the active kernel path. The lib.rs doc comment describes it
  as a "legacy bootloader path" but there is no such path — the kernel uses
  only buddy + TLSF.
- `linked_list.rs` is explicitly labelled "demonstration purposes only" and
  "not production-quality." It does not implement `GlobalAlloc`, `Allocator`,
  or any trait used by the rest of the codebase. It is unreachable from any
  public API.
- `linked_list_allocator = "0.9.0"` is declared as a Cargo dependency but
  nothing in the crate imports it.

**Fix:**

1. Delete `bump.rs` and `linked_list.rs`.
2. Remove `pub mod bump;` and `pub mod linked_list;` from `lib.rs`.
3. Remove `linked_list_allocator` from `abalone/Cargo.toml`.
4. Remove `Locked<A>` from `lib.rs` — it exists only to support
   `BumpAllocator`'s `GlobalAlloc` impl. Once `bump.rs` is gone it is unused.
5. Remove the local `align_up` in `lib.rs` — this duplicates `bitwise::align::align_up`
   which is already a dependency. Callers should use the canonical version.

**Files changed:** delete `abalone/src/bump.rs`, delete
`abalone/src/linked_list.rs`, edit `abalone/src/lib.rs`,
edit `abalone/Cargo.toml`

---

## Issue 4 — `PAGE_SIZE` and `BUDDY_MAX_ORDER` belong in the kernel, not here (ARCHITECTURAL)

**Current state:** `lib.rs` defines:

```rust
pub const PAGE_SIZE:       usize = 4096;
pub const BUDDY_MAX_ORDER: usize = 17;
```

`PAGE_SIZE` is a hardware constant for x86_64 that the entire kernel needs.
`BUDDY_MAX_ORDER` is a tuning parameter for the buddy allocator's static
bitmap size. Both live in a library crate, which means:

- Any other crate that needs `PAGE_SIZE` must depend on `abalone` or
  define its own copy.
- Changing `BUDDY_MAX_ORDER` requires touching the allocator library, not the
  kernel's memory configuration.

**Fix:**

Move both constants to a new file `src/memory/config.rs` in the kernel crate:

```rust
// src/memory/config.rs
/// Base page size for x86_64.
pub const PAGE_SIZE: usize = 4096;

/// Buddy allocator maximum order.
/// 2^17 pages × 4 KiB = 512 MiB addressable per buddy instance.
/// Bitmap cost: ~136 KiB BSS.
pub const BUDDY_MAX_ORDER: usize = 17;
```

In `abalone/src/lib.rs`, replace the definitions with re-exports pointing at
a feature-gated or parameter approach. The simplest interim solution: have
`buddy.rs` accept `PAGE_SIZE` and `BUDDY_MAX_ORDER` as generic const
parameters once Rust const generics stabilise enough, or simply accept that
`abalone` is kernel-specific and the constants are internal.

For now: keep the constants in `abalone` as `pub(crate)` rather than `pub`,
and expose them to the kernel only via `src/memory/config.rs` which re-declares
them. This prevents other hypothetical consumers from depending on abalone's
internal page size definition.

**Files changed:** `abalone/src/lib.rs` (visibility change),
add `src/memory/config.rs`, update `src/memory/mod.rs`

---

## Issue 5 — `AllocStats` is unused outside the buddy and slab (HYGIENE)

**Current state:** `AllocStats` is declared `pub` in `lib.rs` and referenced
in `buddy.rs` and `slab.rs`. Nothing in the kernel currently reads stats at
runtime. The struct is correct and well-designed — it just needs to be
accessible without being a public API commitment.

**Fix:** Change to `pub(crate)` for now. When an observability subsystem
exists and needs to expose stats externally, promote it back to `pub` with
proper documentation.

**Files changed:** `abalone/src/lib.rs`

---

## Issue 6 — `slab.rs` calls `buddy::alloc_pages` directly (STRUCTURAL)

Same coupling problem as TLSF. `slab.rs` calls the module-level
`buddy::alloc_pages` and `buddy::dealloc_pages` free functions which go
through the global `BUDDY` mutex. This means:

- Slab cannot be unit-tested without a live global buddy.
- Multiple slab caches running concurrently all contend on the same mutex
  through an indirect path.

**Fix:** Give `SlabCacheInner` an allocator function pointer or a trait object
at construction time:

```rust
struct SlabCacheInner<T> {
    slab_order:    usize,
    objs_per_slab: usize,
    partial:       *mut SlabHeader,
    stats:         AllocStats,
    alloc_page:    unsafe fn(usize) -> Option<*mut u8>,
    dealloc_page:  unsafe fn(*mut u8, usize),
    _marker:       PhantomData<T>,
}
```

The default construction passes the buddy free functions. Tests can pass
a mock. This is a minor change that pays significant dividends in testability.

**Files changed:** `abalone/src/slab.rs`

---

## Issue 7 — `TlsfInner` tracks `pool_base` and `pool_size` but never uses them (HYGIENE)

```rust
struct TlsfInner {
    ...
    pool_base:  *mut u8,   // written in add_pool, never read
    pool_size:  usize,     // written in add_pool, never read
}
```

These fields exist, presumably, for a future "return pool to buddy on drop"
feature. Until that feature exists they are dead weight and generate
`dead_code` warnings.

**Fix:** Remove them from `TlsfInner`. Add a comment explaining the intended
future use if desired.

**Files changed:** `abalone/src/tlsf.rs`

---

## Execution Order

The issues above should be addressed in this order to minimise the chance of
compiling a broken intermediate state:

1. **Issue 3** — Delete dead code. No functional change, removes noise, makes
   the diff for subsequent changes smaller.

2. **Issue 7** — Remove dead TLSF fields. No functional change.

3. **Issue 5** — Tighten `AllocStats` visibility. No functional change.

4. **Issue 1** — Fix the buddy base problem. This is the only blocking runtime
   bug. Implement `set_base`, update `heap::init`, remove the skip guard from
   `release()`.

5. **Issue 2** — Decouple TLSF from buddy. Requires Issue 1 to be stable first
   so `heap.rs` can be edited cleanly in one pass.

6. **Issue 6** — Decouple slab from buddy. Same rationale.

7. **Issue 4** — Move constants. Do this last since it touches the most files
   and the refactor is least urgent.

---

## Files Touched Summary

| File | Action |
|---|---|
| `abalone/src/buddy.rs` | Add `set_base`, update `add_region` precondition |
| `abalone/src/tlsf.rs` | Add `init_from_ptr`, remove `pool_base`/`pool_size`, remove `buddy` import |
| `abalone/src/slab.rs` | Inject allocator function pointers |
| `abalone/src/lib.rs` | Remove dead modules, tighten visibility, remove `Locked`, remove local `align_up` |
| `abalone/src/bump.rs` | **Delete** |
| `abalone/src/linked_list.rs` | **Delete** |
| `abalone/Cargo.toml` | Remove `linked_list_allocator` dependency |
| `src/memory/heap.rs` | Two-pass buddy init, inject pool ptr to TLSF |
| `src/memory/config.rs` | **Create** — canonical `PAGE_SIZE`, `BUDDY_MAX_ORDER` |
| `src/memory/mod.rs` | Expose `config` module |
| `src/limine_data.rs` | Remove skip guard from `release()` once Issue 1 is fixed |
