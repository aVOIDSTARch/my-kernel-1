// v0.1.1
//! Typed slab allocator backed by the buddy allocator.
//!
//! A [`SlabCache<T>`] manages a collection of slabs. Each slab is one
//! buddy-allocated, power-of-two-page block partitioned into fixed-size `T`
//! slots. Free slots are chained through an embedded index free list stored
//! in the slots themselves (`usize`-wide links, sentinel `usize::MAX`).
//!
//! # Slab memory layout
//!
//! ```text
//!  slab_base  (buddy-aligned to slab_bytes)
//!  ┌─────────────────────────────────────┐
//!  │  SlabHeader                         │  ← always at slab_base
//!  ├─────────────────────────────────────┤  ← slab_base + header_stride
//!  │  slot[0]                            │
//!  │  slot[1]                            │
//!  │  …                                  │
//!  │  slot[capacity-1]                   │
//!  └─────────────────────────────────────┘  ← slab_base + slab_bytes
//! ```
//!
//! `header_stride` is `size_of::<SlabHeader>()` rounded up to
//! `max(align_of::<SlabHeader>(), align_of::<T>())`, ensuring `slot[0]` is
//! correctly aligned for `T`.
//!
//! # Header reconstruction
//!
//! Because the buddy allocator returns blocks aligned to their own size
//! (`slab_bytes = PAGE_SIZE << order`), any object pointer can be masked to
//! recover `slab_base`:
//!
//! ```text
//! slab_base = obj_ptr & !(slab_bytes - 1)
//! ```
//!
//! `SlabHeader` is always at `slab_base`, so no subtraction of a
//! layout-derived offset is needed in `dealloc`. This is the key invariant
//! that makes the previous implementation fragile: it placed the header at
//! the *end* and then tried to reverse-compute `slab_base` from the header
//! pointer using `objs_per_slab * obj_size` — an expression that must agree
//! byte-for-byte with the alignment-mask path used in `dealloc`. Moving the
//! header to the start removes that dependency entirely.
//!
//! # Free list
//!
//! Each free slot's first `size_of::<usize>()` bytes store the index of the
//! next free slot. `usize::MAX` is the end-of-list sentinel. This replaces
//! the original `u16` links, which silently truncated indices for any type
//! large enough to have more than 65 535 slots (impossible at page
//! granularity, but the truncation is still wrong in principle), and which
//! required casting the slot pointer through `*mut u16` regardless of `T`'s
//! alignment.

use core::{
    marker::PhantomData,
    mem,
    ptr::{self, NonNull},
};
use spin::Mutex;
use crate::{AllocStats, PAGE_SIZE};
use crate::buddy;

// ── SlabHeader ────────────────────────────────────────────────────────────────

/// Intrusive doubly-linked list node + per-slab bookkeeping.
/// Stored at `slab_base` (the very start of each buddy allocation).
#[repr(C)]
struct SlabHeader {
    /// Index of the first free slot; `usize::MAX` when the slab is full.
    free_head: usize,
    /// Number of live (allocated) objects currently in this slab.
    in_use:    u32,
    /// Total object slots available in this slab.
    capacity:  u32,
    /// Intrusive partial-list links.
    prev:      *mut SlabHeader,
    next:      *mut SlabHeader,
}

// SAFETY: all access is serialised through `Mutex<SlabCacheInner<T>>`.
unsafe impl Send for SlabHeader {}

impl SlabHeader {
    #[inline] fn is_full(&self)  -> bool { self.in_use == self.capacity }
    #[inline] fn is_empty(&self) -> bool { self.in_use == 0 }
}

// ── Layout helpers ────────────────────────────────────────────────────────────

/// Offset from `slab_base` to `slot[0]`.
///
/// Must satisfy the alignment of both `SlabHeader` (for the header itself at
/// offset 0) and `T` (for the first slot immediately following). We take the
/// larger of the two and round `size_of::<SlabHeader>()` up to it.
const fn header_stride<T>() -> usize {
    let h_align = mem::align_of::<SlabHeader>();
    let t_align = mem::align_of::<T>();
    let align   = if t_align > h_align { t_align } else { h_align };
    (mem::size_of::<SlabHeader>() + align - 1) & !(align - 1)
}

/// Number of `T`-slots that fit in a slab of `PAGE_SIZE << order` bytes.
const fn capacity_for<T>(order: usize) -> usize {
    let slab_bytes = PAGE_SIZE << order;
    let stride     = header_stride::<T>();
    let obj_size   = mem::size_of::<T>();
    if obj_size == 0 || stride >= slab_bytes {
        0
    } else {
        (slab_bytes - stride) / obj_size
    }
}

// ── SlabCacheInner ────────────────────────────────────────────────────────────

struct SlabCacheInner<T> {
    /// Buddy order of backing pages (slab_bytes = PAGE_SIZE << slab_order).
    slab_order:    usize,
    /// Cached result of `capacity_for::<T>(slab_order)`.
    capacity:      usize,
    /// Head of the partial-slab intrusive list (null ⟹ no free slabs).
    partial:       *mut SlabHeader,
    stats:         AllocStats,
    alloc_page:    fn(usize) -> Option<*mut u8>,
    dealloc_page:  unsafe fn(*mut u8, usize),
    _marker:       PhantomData<T>,
}

// SAFETY: serialised through `Mutex`.
unsafe impl<T: Send> Send for SlabCacheInner<T> {}

impl<T> SlabCacheInner<T> {
    const fn new(slab_order: usize) -> Self {
        assert!(
            core::mem::size_of::<T>() >= core::mem::size_of::<usize>(),
            "SlabCache<T>: T must be at least as large as usize; \
            use a wrapper type or increase T's size"
        );
        assert!(
            core::mem::align_of::<T>() <= PAGE_SIZE,
            "SlabCache<T>: T alignment exceeds PAGE_SIZE"
        );
        Self {
            slab_order,
            capacity:     capacity_for::<T>(slab_order),
            partial:      ptr::null_mut(),
            stats: AllocStats {
                total_bytes:   0,
                used_bytes:    0,
                free_bytes:    0,
                alloc_count:   0,
                dealloc_count: 0,
                peak_bytes:    0,
            },
            alloc_page:   buddy::alloc_pages,
            dealloc_page: buddy::dealloc_pages,
            _marker:      PhantomData,
        }
    }

    // ── alloc ─────────────────────────────────────────────────────────────────

    unsafe fn alloc(&mut self) -> Option<NonNull<T>> {
        // Grow if no partial slab is available.
        if self.partial.is_null() {
            self.grow()?;
        }

        let obj_size = mem::size_of::<T>();
        let stride   = header_stride::<T>();

        // Obtain the raw header pointer before forming any reference, so that
        // the subsequent `unlink_partial` call (which takes `&mut self`) does
        // not alias a live `&mut SlabHeader`.
        let header_ptr: *mut SlabHeader = self.partial;

        unsafe {
            let header   = &mut *header_ptr;
            let slot_idx = header.free_head;

            debug_assert_ne!(slot_idx, usize::MAX, "alloc called on full slab");

            // Base of the buddy allocation: header is always here.
            let slab_base = header_ptr as usize;
            let obj_ptr   = (slab_base + stride + slot_idx * obj_size) as *mut T;

            // Advance the free-list head by reading the link stored in the slot.
            let link_ptr      = obj_ptr as *mut usize;
            header.free_head  = ptr::read(link_ptr);
            header.in_use    += 1;

            if header.is_full() {
                // Drop the &mut reference before calling unlink_partial, which
                // borrows self mutably. The raw pointer remains valid.
                self.unlink_partial(header_ptr);
            }

            self.stats.alloc_count += 1;
            self.stats.used_bytes  += obj_size as u64;
            if self.stats.used_bytes > self.stats.peak_bytes {
                self.stats.peak_bytes = self.stats.used_bytes;
            }

            Some(NonNull::new_unchecked(obj_ptr))
        }
    }

    // ── dealloc ───────────────────────────────────────────────────────────────

    unsafe fn dealloc(&mut self, ptr: NonNull<T>) {
        let obj_ptr   = ptr.as_ptr();
        let obj_size  = mem::size_of::<T>();
        let slab_bytes = PAGE_SIZE << self.slab_order;
        let stride    = header_stride::<T>();

        // Recover slab_base by aligning down to slab_bytes.
        // Valid because the buddy guarantees slab_bytes-aligned allocations.
        let slab_base   = (obj_ptr as usize) & !(slab_bytes - 1);
        let header_ptr  = slab_base as *mut SlabHeader;

        unsafe {
            let header = &mut *header_ptr;

            debug_assert!(header.in_use > 0, "dealloc on empty slab");
            debug_assert!(
                (obj_ptr as usize) >= slab_base + stride,
                "obj_ptr precedes object region"
            );
            debug_assert!(
                (obj_ptr as usize) + obj_size <= slab_base + slab_bytes,
                "obj_ptr outside slab bounds"
            );

            let slot_idx = ((obj_ptr as usize) - (slab_base + stride)) / obj_size;
            let was_full = header.is_full();

            // Write the current free-list head into the slot being freed.
            ptr::write(obj_ptr as *mut usize, header.free_head);
            header.free_head  = slot_idx;
            header.in_use    -= 1;

            if was_full {
                self.link_partial(header_ptr);
            } else if header.is_empty() {
                self.unlink_partial(header_ptr);
                (self.dealloc_page)(slab_base as *mut u8, self.slab_order);
                self.stats.total_bytes -= slab_bytes as u64;
                self.stats.free_bytes  -= (self.capacity * obj_size) as u64;
            }
        }

        self.stats.dealloc_count += 1;
        self.stats.used_bytes    -= obj_size as u64;
        self.stats.free_bytes    += obj_size as u64;
    }

    // ── grow ──────────────────────────────────────────────────────────────────

    fn grow(&mut self) -> Option<()> {
        let slab_bytes = PAGE_SIZE << self.slab_order;
        let obj_size   = mem::size_of::<T>();
        let stride     = header_stride::<T>();
        let capacity   = self.capacity;

        assert!(capacity > 0, "SlabCache<T>: zero capacity (T too large for slab order?)");

        let raw       = (self.alloc_page)(self.slab_order)?;
        let slab_base = raw as usize;

        // The buddy must return slab_bytes-aligned memory. If this fires, the
        // slab_base-masking in dealloc will reconstruct the wrong header address.
        if slab_base & !(slab_bytes - 1) != slab_base {
            // Buddy returned a misaligned block. Return it and signal failure
            // rather than proceeding with a slab whose dealloc will reconstruct
            // the wrong header address.
            unsafe { (self.dealloc_page)(raw, self.slab_order) };
            return None;
        }

        // Thread the free list through all slots.
        // slot[i] → i+1, slot[capacity-1] → usize::MAX (end sentinel).
        for i in 0..capacity {
            let slot_ptr = (slab_base + stride + i * obj_size) as *mut usize;
            let next     = if i + 1 < capacity { i + 1 } else { usize::MAX };
            unsafe { ptr::write(slot_ptr, next) };
        }

        // Write the header at slab_base.
        let header_ptr = slab_base as *mut SlabHeader;
        unsafe {
            ptr::write(header_ptr, SlabHeader {
                free_head: 0,
                in_use:    0,
                capacity:  capacity as u32,
                prev:      ptr::null_mut(),
                next:      ptr::null_mut(),
            });
            self.link_partial(header_ptr);
        }

        self.stats.total_bytes += slab_bytes as u64;
        self.stats.free_bytes  += (capacity * obj_size) as u64;
        Some(())
    }

    // ── partial-list management ───────────────────────────────────────────────

    unsafe fn link_partial(&mut self, header: *mut SlabHeader) {
        unsafe {
            (*header).next = self.partial;
            (*header).prev = ptr::null_mut();
            if !self.partial.is_null() {
                (*self.partial).prev = header;
            }
        }
        self.partial = header;
    }

    unsafe fn unlink_partial(&mut self, header: *mut SlabHeader) {
        unsafe {
            let prev = (*header).prev;
            let next = (*header).next;
            if !prev.is_null() { (*prev).next = next; }
            else               { self.partial = next; }
            if !next.is_null() { (*next).prev = prev; }
        }
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Thread-safe typed slab allocator.
///
/// All operations are `O(1)`. The cache is lock-protected by a [`spin::Mutex`];
/// do not call `alloc`/`dealloc` from an interrupt handler that can preempt
/// code already holding the lock.
pub struct SlabCache<T: Send> {
    inner: Mutex<SlabCacheInner<T>>,
}

impl<T: Send> SlabCache<T> {
    /// Create a new cache backed by buddy allocations of `PAGE_SIZE << order`
    /// bytes per slab.
    pub const fn new(slab_order: usize) -> Self {
        Self { inner: Mutex::new(SlabCacheInner::new(slab_order)) }
    }

    /// Allocate one `T`-sized slot. Returns `None` if the backing buddy
    /// allocator cannot satisfy a new slab request.
    pub fn alloc(&self) -> Option<NonNull<T>> {
        unsafe { self.inner.lock().alloc() }
    }

    /// Deallocate a slot previously returned by [`alloc`].
    ///
    /// # Safety
    /// - `ptr` must have originated from `self.alloc()`.
    /// - The object's destructor must have been called before this.
    /// - Must not be called more than once for the same `ptr`.
    pub unsafe fn dealloc(&self, ptr: NonNull<T>) {
        unsafe { self.inner.lock().dealloc(ptr); }
    }

    pub(crate) fn stats(&self) -> AllocStats {
        self.inner.lock().stats
    }
}

// ── Compile-time compatibility guard ─────────────────────────────────────────

/// Assert that `T` is usable as a slab element.
///
/// Call this in a `const` context (e.g., a `static` initialiser) to get a
/// compile-time error rather than a runtime panic.
pub const fn assert_slab_compatible<T>() {
    assert!(
        mem::size_of::<T>() >= mem::size_of::<usize>(),
        "SlabCache<T>: T must be at least as large as usize \
         (free-list link storage requires sizeof(usize) bytes per slot)",
    );
    assert!(
        mem::align_of::<T>() <= PAGE_SIZE,
        "SlabCache<T>: T alignment exceeds PAGE_SIZE",
    );
}
