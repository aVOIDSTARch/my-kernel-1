# Slab Allocator Fix

## The Problem

`alloc()` reconstructs `slab_base` by subtracting from the header pointer:

```rust
let slab_base = header as *mut SlabHeader as usize
    - (self.objs_per_slab * mem::size_of::<T>());
```

`dealloc()` reconstructs it by aligning down:

```rust
let slab_base = (obj_ptr as usize) & !(slab_bytes - 1);
```

These two expressions must produce the same value for `slot_idx` and `header_addr`
to be consistent. They only agree if the buddy allocator returns a block whose
start address is exactly `slab_bytes`-aligned — which it should — but the
subtraction in `alloc()` is the wrong direction to verify that. More critically,
`alloc()` derives `slab_base` from the header, then returns a slot pointer
computed from that base. `dealloc()` derives `slab_base` from the slot pointer,
then computes `header_addr` from that base. If either derivation is off by even
one byte the two sides disagree, `header_addr` in `dealloc()` points at garbage,
and `in_use -= 1` overflows on whatever it finds there.

The fix: use the same derivation everywhere. `& !(slab_bytes - 1)` is the
canonical one because it is independent of object layout arithmetic.

## The Fix

In `SlabCacheInner::alloc()`, replace:

```rust
let slab_base = header as *mut SlabHeader as usize
    - (self.objs_per_slab * mem::size_of::<T>());
```

with:

```rust
let slab_base = (header as *mut SlabHeader as usize) & !(slab_bytes - 1);
let slab_bytes = PAGE_SIZE << self.slab_order;
```

`slab_bytes` is already available in `dealloc()` — add the same binding at the
top of the `alloc()` unsafe block so the mask is well-defined.

The full corrected block:

```rust
unsafe fn alloc(&mut self) -> Option<NonNull<T>> {
    if self.partial.is_null() {
        self.grow()?;
    }

    unsafe {
        let slab_bytes = PAGE_SIZE << self.slab_order;   // <-- add this
        let header     = &mut *self.partial;
        let slab_base  = (header as *mut SlabHeader as usize) & !(slab_bytes - 1); // <-- fix
        let obj_size   = mem::size_of::<T>();
        let slot_idx   = header.free_head as usize;

        let slot_ptr     = (slab_base + slot_idx * obj_size) as *mut u16;
        header.free_head = ptr::read(slot_ptr);
        header.in_use   += 1;

        if header.is_full() {
            self.unlink_partial(header as *mut SlabHeader);
        }

        let obj_ptr = slot_ptr as *mut T;
        self.stats.alloc_count += 1;
        self.stats.used_bytes  += obj_size as u64;
        if self.stats.used_bytes > self.stats.peak_bytes {
            self.stats.peak_bytes = self.stats.used_bytes;
        }

        Some(NonNull::new_unchecked(obj_ptr))
    }
}
```

## Recommended Assertion in `grow()`

Add this after computing `header_addr` to catch any future type or order that
overflows the slab at compile time or early in testing:

```rust
debug_assert!(
    header_addr + mem::size_of::<SlabHeader>() <= slab_base + slab_bytes,
    "SlabHeader overflows slab: objs_per_slab={} obj_size={} header_size={}",
    self.objs_per_slab,
    mem::size_of::<T>(),
    mem::size_of::<SlabHeader>(),
);
```
