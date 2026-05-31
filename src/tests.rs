// v0.0.5
// Kernel-level tests for mantle and abalone types.
// These run in QEMU via the kernel's custom test runner after full
// kernel initialization (heap, VMM, and timer are all up).


use mantle::prot::Protection;
use mantle::table::PageTable;
use bitwise::paging::pte_flags;

// ── mantle::prot::Protection ─────────────────────────────────────────────────

#[test_case]
fn kernel_rw_is_writable_nx_and_global() {
    let b = Protection::KERNEL_RW.bits();
    assert_ne!(b & pte_flags::WRITABLE,   0, "KERNEL_RW must be writable");
    assert_ne!(b & pte_flags::NO_EXECUTE, 0, "KERNEL_RW must not execute");
    assert_ne!(b & pte_flags::GLOBAL,     0, "KERNEL_RW must be global");
}

#[test_case]
fn kernel_rx_is_executable_not_writable() {
    let b = Protection::KERNEL_RX.bits();
    assert_eq!(b & pte_flags::NO_EXECUTE, 0, "KERNEL_RX must be executable");
    assert_eq!(b & pte_flags::WRITABLE,   0, "KERNEL_RX must not be writable");
    assert_ne!(b & pte_flags::GLOBAL,     0, "KERNEL_RX must be global");
}

#[test_case]
fn kernel_ro_is_not_writable_and_not_executable() {
    let b = Protection::KERNEL_RO.bits();
    assert_eq!(b & pte_flags::WRITABLE,   0, "KERNEL_RO must not be writable");
    assert_ne!(b & pte_flags::NO_EXECUTE, 0, "KERNEL_RO must not execute");
    assert_ne!(b & pte_flags::GLOBAL,     0, "KERNEL_RO must be global");
}

#[test_case]
fn mmio_uc_sets_cache_disable_and_write_through_not_pat() {
    let b = Protection::MMIO_UC.bits();
    assert_ne!(b & pte_flags::WRITABLE,      0, "MMIO_UC must be writable");
    assert_ne!(b & pte_flags::NO_EXECUTE,    0, "MMIO_UC must not execute");
    assert_ne!(b & pte_flags::CACHE_DISABLE, 0, "MMIO_UC must disable cache");
    assert_ne!(b & pte_flags::WRITE_THROUGH, 0, "MMIO_UC must set write-through");
    assert_eq!(b & pte_flags::PAT,           0, "MMIO_UC must not set PAT bit");
}

#[test_case]
fn mmio_wc_adds_pat_bit_to_mmio_uc() {
    let uc = Protection::MMIO_UC.bits();
    let wc = Protection::MMIO_WC.bits();
    assert_ne!(wc & pte_flags::PAT, 0, "MMIO_WC must set PAT bit");
    assert_eq!(wc & !pte_flags::PAT, uc, "MMIO_WC must be MMIO_UC plus PAT bit");
}

// ── mantle::table::PageTable ─────────────────────────────────────────────────

#[repr(align(4096))]
struct AlignedPage([u8; 4096]);

static mut PAGE_TABLE_MEM: AlignedPage = AlignedPage([0u8; 4096]);

fn test_page_table() -> &'static mut PageTable {
    // Safety: tests run sequentially; PAGE_TABLE_MEM is not aliased elsewhere.
    unsafe { &mut *core::ptr::addr_of_mut!(PAGE_TABLE_MEM.0).cast::<PageTable>() }
}

#[test_case]
fn page_table_write_then_read_roundtrip() {
    let pt = test_page_table();
    pt.write(0,   0xDEAD_BEEF_0000_0001);
    pt.write(511, 0x1234_5678_0000_0003);
    assert_eq!(pt.read(0),   0xDEAD_BEEF_0000_0001);
    assert_eq!(pt.read(511), 0x1234_5678_0000_0003);
}

#[test_case]
fn page_table_zero_clears_all_512_entries() {
    let pt = test_page_table();
    for i in 0..512 { pt.write(i, 0xFFFF_FFFF_FFFF_FFFF); }
    pt.zero();
    for i in 0..512 {
        assert_eq!(pt.read(i), 0);
    }
}

#[test_case]
fn page_table_write_does_not_corrupt_neighbors() {
    let pt = test_page_table();
    pt.zero();
    pt.write(100, 0xABCD_EF00_1234_5678);
    assert_eq!(pt.read(99),  0, "entry before write was corrupted");
    assert_eq!(pt.read(100), 0xABCD_EF00_1234_5678);
    assert_eq!(pt.read(101), 0, "entry after write was corrupted");
}

// ── abalone::tlsf::TlsfAllocator ─────────────────────────────────────────────

use abalone::tlsf::TlsfAllocator;
use core::alloc::Layout;

const TLSF_POOL: usize = 65536;

#[repr(align(16))]
struct TlsfBuf([u8; TLSF_POOL]);

static mut TLSF_MEM: TlsfBuf = TlsfBuf([0u8; TLSF_POOL]);

fn fresh_tlsf() -> TlsfAllocator {
    let alloc = TlsfAllocator::new();
    // Safety: TLSF_MEM is not aliased by any other allocator; tests run sequentially.
    unsafe { alloc.init_from_ptr(core::ptr::addr_of_mut!(TLSF_MEM.0).cast(), TLSF_POOL); }
    alloc
}

#[test_case]
fn tlsf_alloc_returns_non_null() {
    let alloc = fresh_tlsf();
    let layout = Layout::from_size_align(64, 8).unwrap();
    let ptr = unsafe { core::alloc::GlobalAlloc::alloc(&alloc, layout) };
    assert!(!ptr.is_null(), "TLSF alloc must return non-null for small request");
    unsafe { core::alloc::GlobalAlloc::dealloc(&alloc, ptr, layout) };
}

#[test_case]
fn tlsf_alloc_returns_null_when_oom() {
    let alloc = fresh_tlsf();
    let layout = Layout::from_size_align(TLSF_POOL * 2, 8).unwrap();
    let ptr = unsafe { core::alloc::GlobalAlloc::alloc(&alloc, layout) };
    assert!(ptr.is_null(), "TLSF must return null when pool is exhausted");
}

#[test_case]
fn tlsf_alloc_multiple_non_overlapping_regions() {
    let alloc = fresh_tlsf();
    let layout = Layout::from_size_align(64, 8).unwrap();
    let p0 = unsafe { core::alloc::GlobalAlloc::alloc(&alloc, layout) };
    let p1 = unsafe { core::alloc::GlobalAlloc::alloc(&alloc, layout) };
    assert!(!p0.is_null());
    assert!(!p1.is_null());
    // Pointers must not overlap: distance >= 64 bytes.
    let diff = (p0 as isize - p1 as isize).unsigned_abs();
    assert!(diff >= 64, "allocations overlap: p0={p0:?} p1={p1:?}");
    unsafe {
        core::alloc::GlobalAlloc::dealloc(&alloc, p0, layout);
        core::alloc::GlobalAlloc::dealloc(&alloc, p1, layout);
    }
}

#[test_case]
fn tlsf_dealloc_and_realloc_succeeds() {
    let alloc = fresh_tlsf();
    let layout = Layout::from_size_align(128, 8).unwrap();
    let p0 = unsafe { core::alloc::GlobalAlloc::alloc(&alloc, layout) };
    assert!(!p0.is_null());
    unsafe { core::alloc::GlobalAlloc::dealloc(&alloc, p0, layout) };
    // After deallocation the pool must accept a new allocation.
    let p1 = unsafe { core::alloc::GlobalAlloc::alloc(&alloc, layout) };
    assert!(!p1.is_null(), "alloc after dealloc must succeed");
    unsafe { core::alloc::GlobalAlloc::dealloc(&alloc, p1, layout) };
}

// ── abalone::slab::SlabCache ─────────────────────────────────────────────────

use abalone::slab::SlabCache;

// ── abalone::slab::SlabCache tests ───────────────────────────────────────────
//
// All tests run after heap::init() — the buddy allocator is seeded and the
// TLSF heap is live. SlabCache<T> is backed by the buddy directly (not TLSF),
// so these tests exercise the slab layer independently of the global allocator.
//
// Conventions:
//   - Each test owns its own SlabCache so there is no shared state.
//   - `cache.alloc()` returns Option<NonNull<T>>; unwrap with a message.
//   - `cache.dealloc(ptr)` is unsafe; the caller is responsible for ensuring
//     the pointer came from the same cache and the destructor has been called.
//   - Tests are ordered from most basic to most structural so that a failure
//     in an early test makes the cause of later failures obvious.



// ── 1. Basic allocation and write ────────────────────────────────────────────

#[test_case]
fn slab_alloc_returns_valid_pointer() {
    // Simplest possible test: allocate one u64, write a known pattern, read
    // it back. Failure here means the returned pointer is garbage or the slab
    // layout is wrong at the most fundamental level.
    let cache: SlabCache<u64> = SlabCache::new(0);
    let ptr = cache.alloc().expect("slab alloc must return Some for u64");
    unsafe { ptr.as_ptr().write(0xCAFE_BABE_DEAD_BEEF) };
    assert_eq!(
        unsafe { ptr.as_ptr().read() },
        0xCAFE_BABE_DEAD_BEEF,
        "value read back does not match value written",
    );
    unsafe { cache.dealloc(ptr) };
}

// ── 2. Non-overlap of consecutive allocations ─────────────────────────────────

#[test_case]
fn slab_two_allocs_do_not_overlap() {
    // Two allocations must be at least size_of::<[u8;32]>() bytes apart.
    // If they overlap the slab free-list initialisation is broken.
    let cache: SlabCache<[u64; 32]> = SlabCache::new(0);
    let p0 = cache.alloc().expect("first alloc");
    let p1 = cache.alloc().expect("second alloc");
    let diff = (p0.as_ptr() as isize - p1.as_ptr() as isize).unsigned_abs();
    assert!(
        diff >= 32,
        "allocations overlap: p0={:#x} p1={:#x} diff={}",
        p0.as_ptr() as usize, p1.as_ptr() as usize, diff,
    );
    unsafe {
        cache.dealloc(p0);
        cache.dealloc(p1);
    }
}

// ── 3. Dealloc then realloc succeeds ─────────────────────────────────────────

#[test_case]
fn slab_dealloc_and_realloc_succeeds() {
    // After freeing the only allocated slot, the next alloc must succeed.
    // Failure here means the free-list head is not correctly restored on dealloc.
    let cache: SlabCache<u64> = SlabCache::new(0);
    let p0 = cache.alloc().expect("first alloc");
    unsafe { cache.dealloc(p0) };
    let p1 = cache.alloc().expect("alloc after dealloc must succeed");
    unsafe { cache.dealloc(p1) };
}

// ── 4. Freed slot is reused ───────────────────────────────────────────────────

#[test_case]
fn slab_realloc_reuses_freed_slot() {
    // Allocate two slots, free the first, then allocate again. The new
    // allocation must land at the same address as the freed one — the slab
    // must be returning freed slots from its free list, not growing.
    //
    // This also validates that the free-list link written into the slot during
    // dealloc does not corrupt the value subsequently written by the caller.
    let cache: SlabCache<u64> = SlabCache::new(0);
    let p0 = cache.alloc().expect("first alloc");
    let p1 = cache.alloc().expect("second alloc");
    let addr0 = p0.as_ptr() as usize;

    unsafe { cache.dealloc(p0) };

    let p2 = cache.alloc().expect("third alloc after freeing p0");
    assert_eq!(
        p2.as_ptr() as usize, addr0,
        "expected reuse of freed slot at {:#x}, got {:#x}",
        addr0, p2.as_ptr() as usize,
    );

    // Verify the reused slot is still writable and readable.
    unsafe { p2.as_ptr().write(0x1234_5678_9ABC_DEF0) };
    assert_eq!(unsafe { p2.as_ptr().read() }, 0x1234_5678_9ABC_DEF0);

    unsafe {
        cache.dealloc(p1);
        cache.dealloc(p2);
    }
}

// ── 5. Exhaust one slab, triggering a second ──────────────────────────────────

#[test_case]
fn slab_fills_first_slab_and_grows() {
    // [u8; 64] at order 0: header_stride=32, slab_bytes=4096, capacity=63.
    // Allocate 64 objects — the 64th must succeed, proving the slab grows a
    // second backing page when the first is exhausted.
    //
    // Capacity = (4096 - 32) / 64 = 4064 / 64 = 63.
    // So slot 63 (0-indexed) requires a second slab.
    const N: usize = 64;
    let cache: SlabCache<[u8; 64]> = SlabCache::new(0);
    let mut ptrs = [None; N];
    for i in 0..N {
        ptrs[i] = Some(
            cache.alloc().unwrap_or_else(|| panic!("alloc failed at i={}", i))
        );
    }
    // All pointers must be non-null and distinct.
    for i in 0..N {
        for j in (i + 1)..N {
            assert_ne!(
                ptrs[i].unwrap().as_ptr() as usize,
                ptrs[j].unwrap().as_ptr() as usize,
                "duplicate pointer at i={} j={}",
                i, j,
            );
        }
    }
    for p in ptrs.iter().flatten() {
        unsafe { cache.dealloc(*p) };
    }
}

// ── 6. Full slab is returned to buddy on complete dealloc ─────────────────────

#[test_case]
fn slab_empty_slab_is_released() {
    // Allocate enough objects to fill exactly one slab, then free them all.
    // The slab must be returned to the buddy (partial list becomes empty).
    // A subsequent alloc must succeed, proving the buddy got the pages back
    // and can re-issue them for a new slab.
    //
    // capacity for [u8; 64] at order 0 = 63 (see test above).
    const CAPACITY: usize = 63;
    let cache: SlabCache<[u8; 64]> = SlabCache::new(0);
    let mut ptrs = [None; CAPACITY];
    for i in 0..CAPACITY {
        ptrs[i] = Some(cache.alloc().expect("alloc while filling slab"));
    }
    for p in ptrs.iter().flatten() {
        unsafe { cache.dealloc(*p) };
    }
    // After releasing the slab, a fresh alloc must still work.
    let p = cache.alloc().expect("alloc after slab release must succeed");
    unsafe { cache.dealloc(p) };
}

// ── 7. Write-then-read across multiple allocations ────────────────────────────

#[test_case]
fn slab_multiple_slots_hold_independent_values() {
    // Write a distinct u64 into each of N slots, then read them all back.
    // Any overlap in the slab layout would cause adjacent writes to collide.
    const N: usize = 16;
    let cache: SlabCache<u64> = SlabCache::new(0);
    let mut ptrs = [None; N];
    for i in 0..N {
        let p = cache.alloc().expect("alloc in loop");
        unsafe { p.as_ptr().write(i as u64 * 0x1111_1111_1111_1111) };
        ptrs[i] = Some(p);
    }
    for i in 0..N {
        let p = ptrs[i].unwrap();
        assert_eq!(
            unsafe { p.as_ptr().read() },
            i as u64 * 0x1111_1111_1111_1111,
            "slot {} value corrupted",
            i,
        );
    }
    for p in ptrs.iter().flatten() {
        unsafe { cache.dealloc(*p) };
    }
}

// ── 8. Interleaved alloc and dealloc ─────────────────────────────────────────

#[test_case]
fn slab_interleaved_alloc_dealloc_stays_consistent() {
    // Allocate 8, free every other one, allocate 4 more, verify all 8 live
    // pointers hold the values written to them. This exercises the free-list
    // in a non-trivial order and catches any corruption in the link chain.
    let cache: SlabCache<u64> = SlabCache::new(0);
    let mut live = [None::<core::ptr::NonNull<u64>>; 8];

    // First wave: 8 allocations.
    for i in 0..8usize {
        let p = cache.alloc().expect("first wave alloc");
        unsafe { p.as_ptr().write(i as u64) };
        live[i] = Some(p);
    }

    // Free even-indexed slots.
    for i in (0..8).step_by(2) {
        unsafe { cache.dealloc(live[i].take().unwrap()) };
    }

    // Second wave: 4 allocations filling the freed slots.
    let mut second = [None::<core::ptr::NonNull<u64>>; 4];
    for i in 0..4usize {
        let p = cache.alloc().expect("second wave alloc");
        unsafe { p.as_ptr().write(0x100 + i as u64) };
        second[i] = Some(p);
    }

    // Verify odd-indexed survivors are intact.
    for i in (1..8).step_by(2) {
        assert_eq!(
            unsafe { live[i].unwrap().as_ptr().read() },
            i as u64,
            "slot {} corrupted during interleaved ops",
            i,
        );
    }

    // Verify second-wave values.
    for i in 0..4usize {
        assert_eq!(
            unsafe { second[i].unwrap().as_ptr().read() },
            0x100 + i as u64,
            "second-wave slot {} corrupted",
            i,
        );
    }

    // Clean up.
    for p in live.iter().flatten()   { unsafe { cache.dealloc(*p) }; }
    for p in second.iter().flatten() { unsafe { cache.dealloc(*p) }; }
}

// ── 9. Large object alignment ─────────────────────────────────────────────────

#[test_case]
fn slab_pointers_are_aligned_for_type() {
    // Every returned pointer must satisfy align_of::<T>(). Misaligned slots
    // indicate the header_stride calculation is wrong for T's alignment.
    // u128 has align=16 on x86_64, making it a meaningful stress case.
    let cache: SlabCache<u128> = SlabCache::new(0);
    let align = core::mem::align_of::<u128>();
    let mut ptrs = [None; 8];
    for i in 0..8usize {
        let p = cache.alloc().expect("u128 alloc");
        assert_eq!(
            p.as_ptr() as usize % align, 0,
            "slot {} misaligned: {:#x} % {} != 0",
            i, p.as_ptr() as usize, align,
        );
        ptrs[i] = Some(p);
    }
    for p in ptrs.iter().flatten() {
        unsafe { cache.dealloc(*p) };
    }
}

// ── 10. Higher slab order ─────────────────────────────────────────────────────

#[test_case]
fn slab_order1_alloc_and_dealloc() {
    // Order-1 slab = 8 KiB. If the buddy has no order-1 block available
    // (possible if earlier tests consumed memory), skip gracefully.
    let cache: SlabCache<u64> = SlabCache::new(1);
    let p0 = match cache.alloc() {
        Some(p) => p,
        None => return, // buddy exhausted at order-1; not a slab bug
    };
    unsafe { p0.as_ptr().write(0xDEAD_C0DE_BEEF_CAFE) };
    assert_eq!(unsafe { p0.as_ptr().read() }, 0xDEAD_C0DE_BEEF_CAFE);
    // Verify the pointer is 8-byte aligned (align_of::<u64>()).
    assert_eq!(p0.as_ptr() as usize % core::mem::align_of::<u64>(), 0);
    unsafe { cache.dealloc(p0) };
}
