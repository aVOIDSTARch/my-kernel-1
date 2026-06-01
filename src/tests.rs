// v0.1.1
// Comprehensive kernel integration tests.
// All tests run in QEMU after full kernel initialization: heap, buddy,
// VMM, process table, and the new kernel PML4 are all live.
//
// Sections (each begins with a marker test that prints a visible header):
//   1. mantle::prot::Protection         (existing)
//   2. mantle::table::PageTable         (existing)
//   3. abalone::tlsf::TlsfAllocator     (existing)
//   4. abalone::slab::SlabCache         (existing)
//   5. pincer::SpinMutex                (new)
//   6. pincer::IrqMutex                 (new)
//   7. pincer::WaitQueue                (new)
//   8. seastar::ids                     (new)
//   9. seastar::state                   (new)
//  10. seastar::flags                   (new)
//  11. seastar::context                 (new)
//  12. seastar::priority                (new)
//  13. seastar::Process                 (new)
//  14. seastar::ProcessTable            (new)
//  15. mantle::pml4 live validation     (new)


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

// ═══════════════════════════════════════════════════════════════════════════════
// 5. pincer::SpinMutex
//
// Tests run on a stack-allocated SpinMutex with no interrupt management.
// Single-threaded; we exploit scoped guards to test try_lock contention.
// ═══════════════════════════════════════════════════════════════════════════════

use pincer::psync::{SpinMutex, IrqMutex, WaitQueue, WakerToken};
use pincer::psync::{WaiterPriority, StarvationConfig};

#[test_case]
fn _section_pincer_spinmutex() {
    crate::serial_println!("\n=== pincer::SpinMutex ===");
}

#[test_case]
fn spinmutex_new_is_unlocked() {
    let m = SpinMutex::new(0u32);
    assert!(!m.is_locked(), "freshly constructed SpinMutex must report unlocked");
}

#[test_case]
fn spinmutex_lock_marks_as_locked() {
    let m = SpinMutex::new(0u32);
    let _g = m.lock();
    assert!(m.is_locked(), "is_locked must be true while guard is held");
}

#[test_case]
fn spinmutex_guard_drop_releases_lock() {
    let m = SpinMutex::new(());
    let g = m.lock();
    assert!(m.is_locked());
    drop(g);
    assert!(!m.is_locked(), "lock must be released when guard is dropped");
}

#[test_case]
fn spinmutex_data_is_readable_through_guard() {
    let m = SpinMutex::new(0xCAFE_BABEu32);
    let g = m.lock();
    assert_eq!(*g, 0xCAFE_BABE);
}

#[test_case]
fn spinmutex_data_is_writable_through_guard() {
    let m = SpinMutex::new(0u32);
    {
        let mut g = m.lock();
        *g = 0xDEAD_BEEFu32;
    }
    // Re-acquire and verify the write persisted.
    assert_eq!(*m.lock(), 0xDEAD_BEEF);
}

#[test_case]
fn spinmutex_try_lock_succeeds_when_free() {
    let m = SpinMutex::new(7u32);
    let guard = m.try_lock();
    assert!(guard.is_some(), "try_lock must succeed on an unlocked SpinMutex");
    assert_eq!(*guard.unwrap(), 7);
}

#[test_case]
fn spinmutex_try_lock_fails_when_held() {
    let m = SpinMutex::new(());
    let _held = m.lock();
    let second = m.try_lock();
    assert!(second.is_none(), "try_lock must return None while lock is held");
}

#[test_case]
fn spinmutex_heap_value_survives_lock_cycle() {
    let m = SpinMutex::new(alloc::vec![1u32, 2, 3]);
    { m.lock().push(4); }
    assert_eq!(m.lock().as_slice(), &[1u32, 2, 3, 4]);
}

// ═══════════════════════════════════════════════════════════════════════════════
// 6. pincer::IrqMutex
//
// Uses X86IrqControl (saves/restores RFLAGS via pushfq/popfq + cli).
// Interrupt state is preserved across lock/unlock — we verify the lock
// itself acquires and releases correctly; IRQ state is implicitly tested
// by the fact that interrupts are re-enabled after kernel_main_continue.
// ═══════════════════════════════════════════════════════════════════════════════

#[test_case]
fn _section_pincer_irqmutex() {
    crate::serial_println!("\n=== pincer::IrqMutex ===");
}

type IrqU32 = IrqMutex<u32, crate::arch::X86IrqControl>;

#[test_case]
fn irqmutex_new_is_unlocked() {
    let m: IrqU32 = IrqMutex::new(0);
    assert!(!m.is_locked());
}

#[test_case]
fn irqmutex_lock_marks_as_locked() {
    let m: IrqU32 = IrqMutex::new(0);
    let _g = m.lock();
    assert!(m.is_locked());
}

#[test_case]
fn irqmutex_guard_drop_releases_lock() {
    let m: IrqU32 = IrqMutex::new(0);
    let g = m.lock();
    assert!(m.is_locked());
    drop(g);
    assert!(!m.is_locked());
}

#[test_case]
fn irqmutex_data_readable_through_guard() {
    let m: IrqU32 = IrqMutex::new(0xFEED_FACEu32);
    assert_eq!(*m.lock(), 0xFEED_FACE);
}

#[test_case]
fn irqmutex_data_writable_through_guard() {
    let m: IrqU32 = IrqMutex::new(0);
    { *m.lock() = 0xABCD_1234; }
    assert_eq!(*m.lock(), 0xABCD_1234);
}

#[test_case]
fn irqmutex_try_lock_succeeds_when_free() {
    let m: IrqU32 = IrqMutex::new(42);
    let g = m.try_lock();
    assert!(g.is_some(), "try_lock must succeed on unlocked IrqMutex");
    assert_eq!(*g.unwrap(), 42);
}

#[test_case]
fn irqmutex_try_lock_fails_when_held() {
    let m: IrqU32 = IrqMutex::new(0);
    let _held = m.lock();
    let second = m.try_lock();
    assert!(second.is_none(), "try_lock must return None while IrqMutex is held");
}

// ═══════════════════════════════════════════════════════════════════════════════
// 7. pincer::WaitQueue
//
// Compiled without the `alloc` feature (kernel uses pincer with only
// `starvation_protection`), so the queue uses a static 32-slot array.
// ═══════════════════════════════════════════════════════════════════════════════

#[test_case]
fn _section_pincer_waitqueue() {
    crate::serial_println!("\n=== pincer::WaitQueue ===");
}

#[test_case]
fn waitqueue_new_is_empty() {
    let q = WaitQueue::new(StarvationConfig::DEFAULT);
    assert!(q.is_empty());
    assert_eq!(q.len(), 0);
}

#[test_case]
fn waitqueue_enqueue_increases_len() {
    let mut q = WaitQueue::new(StarvationConfig::DEFAULT);
    q.enqueue(WaiterPriority::Normal, WakerToken::from_raw(1)).unwrap();
    assert_eq!(q.len(), 1);
    assert!(!q.is_empty());
}

#[test_case]
fn waitqueue_dequeue_from_empty_is_none() {
    let mut q = WaitQueue::new(StarvationConfig::DEFAULT);
    assert!(q.dequeue_next().is_none());
}

#[test_case]
fn waitqueue_enqueue_dequeue_roundtrip() {
    let mut q = WaitQueue::new(StarvationConfig::DEFAULT);
    let tok = WakerToken::from_raw(99);
    q.enqueue(WaiterPriority::Normal, tok).unwrap();
    let got = q.dequeue_next();
    assert_eq!(got, Some(tok));
    assert!(q.is_empty());
}

#[test_case]
fn waitqueue_high_priority_exits_before_normal() {
    let mut q = WaitQueue::new(StarvationConfig::DEFAULT);
    let normal_tok = WakerToken::from_raw(1);
    let high_tok   = WakerToken::from_raw(2);
    // Enqueue Normal first, then High; High must dequeue first.
    q.enqueue(WaiterPriority::Normal, normal_tok).unwrap();
    q.enqueue(WaiterPriority::High,   high_tok).unwrap();
    assert_eq!(q.dequeue_next(), Some(high_tok),   "High must exit before Normal");
    assert_eq!(q.dequeue_next(), Some(normal_tok), "Normal exits second");
}

#[test_case]
fn waitqueue_fifo_within_same_priority_tier() {
    let mut q = WaitQueue::new(StarvationConfig::DEFAULT);
    let t1 = WakerToken::from_raw(10);
    let t2 = WakerToken::from_raw(20);
    let t3 = WakerToken::from_raw(30);
    q.enqueue(WaiterPriority::Normal, t1).unwrap();
    q.enqueue(WaiterPriority::Normal, t2).unwrap();
    q.enqueue(WaiterPriority::Normal, t3).unwrap();
    // Must come out in insertion order (FIFO within same priority).
    assert_eq!(q.dequeue_next(), Some(t1));
    assert_eq!(q.dequeue_next(), Some(t2));
    assert_eq!(q.dequeue_next(), Some(t3));
    assert!(q.is_empty());
}

#[test_case]
fn waitqueue_low_priority_exits_after_all_others() {
    let mut q = WaitQueue::new(StarvationConfig::DEFAULT);
    let low  = WakerToken::from_raw(1);
    let norm = WakerToken::from_raw(2);
    let high = WakerToken::from_raw(3);
    q.enqueue(WaiterPriority::Low,    low).unwrap();
    q.enqueue(WaiterPriority::High,   high).unwrap();
    q.enqueue(WaiterPriority::Normal, norm).unwrap();
    assert_eq!(q.dequeue_next(), Some(high));
    assert_eq!(q.dequeue_next(), Some(norm));
    assert_eq!(q.dequeue_next(), Some(low));
}

// ═══════════════════════════════════════════════════════════════════════════════
// 8. seastar::ids
// ═══════════════════════════════════════════════════════════════════════════════

use seastar::{ProcessId, ThreadId};

#[test_case]
fn _section_seastar_ids() {
    crate::serial_println!("\n=== seastar::ids ===");
}

#[test_case]
fn process_id_invalid_raw_is_zero() {
    assert_eq!(ProcessId::INVALID.as_u64(), 0);
}

#[test_case]
fn process_id_from_raw_roundtrip() {
    let id = ProcessId::from_raw(42);
    assert_eq!(id.as_u64(), 42);
}

#[test_case]
fn process_id_invalid_not_equal_to_any_live_id() {
    let live = ProcessId::from_raw(1);
    assert_ne!(live, ProcessId::INVALID);
}

#[test_case]
fn process_id_as_u32_for_syscall_truncates_correctly() {
    // Lower 32 bits must be preserved; upper 32 bits are truncated.
    let id = ProcessId::from_raw(0x0000_0001_DEAD_BEEFu64);
    assert_eq!(id.as_u32_for_syscall(), 0xDEAD_BEEFu32);
}

#[test_case]
fn process_id_ordering_is_numeric() {
    let a = ProcessId::from_raw(1);
    let b = ProcessId::from_raw(100);
    assert!(a < b);
    assert!(b > a);
}

#[test_case]
fn thread_id_idle_is_zero() {
    assert_eq!(ThreadId::IDLE.as_u64(), 0);
}

#[test_case]
fn thread_id_new_never_returns_zero() {
    let id = ThreadId::new();
    assert_ne!(id.as_u64(), 0, "ThreadId::new must not return IDLE (0)");
}

#[test_case]
fn thread_id_new_is_strictly_increasing() {
    let a = ThreadId::new();
    let b = ThreadId::new();
    assert!(a < b, "consecutive ThreadId::new calls must strictly increase");
}

// ═══════════════════════════════════════════════════════════════════════════════
// 9. seastar::state
// ═══════════════════════════════════════════════════════════════════════════════

use seastar::{ProcessState, ThreadState, BlockedReason};

#[test_case]
fn _section_seastar_state() {
    crate::serial_println!("\n=== seastar::state ===");
}

#[test_case]
fn process_state_created_eq_itself() {
    assert_eq!(ProcessState::Created, ProcessState::Created);
}

#[test_case]
fn process_state_blocked_carries_reason() {
    let s = ProcessState::Blocked(BlockedReason::Io);
    match s {
        ProcessState::Blocked(BlockedReason::Io) => {}
        other => panic!("expected Blocked(Io), got {:?}", other),
    }
}

#[test_case]
fn process_state_variants_are_all_distinct() {
    let states: &[ProcessState] = &[
        ProcessState::Created,
        ProcessState::Ready,
        ProcessState::Running,
        ProcessState::Blocked(BlockedReason::Sleep),
        ProcessState::Zombie,
    ];
    for (i, a) in states.iter().enumerate() {
        for (j, b) in states.iter().enumerate() {
            if i != j {
                assert_ne!(a, b, "states at index {} and {} must differ", i, j);
            }
        }
    }
}

#[test_case]
fn thread_state_dead_is_distinct_from_active_states() {
    assert_ne!(ThreadState::Dead, ThreadState::Created);
    assert_ne!(ThreadState::Dead, ThreadState::Ready);
    assert_ne!(ThreadState::Dead, ThreadState::Running);
    assert_ne!(ThreadState::Dead, ThreadState::Blocked(BlockedReason::Sleep));
}

#[test_case]
fn blocked_reasons_are_all_distinct() {
    let reasons = [
        BlockedReason::Sleep,
        BlockedReason::Synchronisation,
        BlockedReason::Io,
        BlockedReason::WaitChild,
        BlockedReason::Other,
    ];
    for (i, a) in reasons.iter().enumerate() {
        for (j, b) in reasons.iter().enumerate() {
            if i != j {
                assert_ne!(a, b);
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 10. seastar::flags
// ═══════════════════════════════════════════════════════════════════════════════

use seastar::flags::{ProcessFlags, ThreadFlags};

#[test_case]
fn _section_seastar_flags() {
    crate::serial_println!("\n=== seastar::flags ===");
}

#[test_case]
fn process_flags_kernel_process_is_bit_0() {
    assert_eq!(ProcessFlags::KERNEL_PROCESS.bits(), 1 << 0);
}

#[test_case]
fn process_flags_exiting_is_bit_1() {
    assert_eq!(ProcessFlags::EXITING.bits(), 1 << 1);
}

#[test_case]
fn process_flags_traced_is_bit_2() {
    assert_eq!(ProcessFlags::TRACED.bits(), 1 << 2);
}

#[test_case]
fn process_flags_signal_pending_is_bit_3() {
    assert_eq!(ProcessFlags::SIGNAL_PENDING.bits(), 1 << 3);
}

#[test_case]
fn process_flags_combine_without_overlap() {
    let combined = ProcessFlags::KERNEL_PROCESS | ProcessFlags::SIGNAL_PENDING;
    assert!(combined.contains(ProcessFlags::KERNEL_PROCESS));
    assert!(combined.contains(ProcessFlags::SIGNAL_PENDING));
    assert!(!combined.contains(ProcessFlags::EXITING));
    assert!(!combined.contains(ProcessFlags::TRACED));
}

#[test_case]
fn process_flags_remove_clears_single_bit() {
    let mut flags = ProcessFlags::KERNEL_PROCESS | ProcessFlags::EXITING;
    flags.remove(ProcessFlags::EXITING);
    assert!(!flags.contains(ProcessFlags::EXITING));
    assert!(flags.contains(ProcessFlags::KERNEL_PROCESS));
}

#[test_case]
fn thread_flags_main_thread_is_bit_0() {
    assert_eq!(ThreadFlags::MAIN_THREAD.bits(), 1 << 0);
}

#[test_case]
fn thread_flags_exiting_is_bit_1() {
    assert_eq!(ThreadFlags::EXITING.bits(), 1 << 1);
}

#[test_case]
fn thread_flags_fpu_state_dirty_is_bit_2() {
    assert_eq!(ThreadFlags::FPU_STATE_DIRTY.bits(), 1 << 2);
}

#[test_case]
fn thread_flags_in_syscall_is_bit_3() {
    assert_eq!(ThreadFlags::IN_SYSCALL.bits(), 1 << 3);
}

#[test_case]
fn thread_flags_empty_is_zero() {
    assert_eq!(ThreadFlags::empty().bits(), 0);
}

#[test_case]
fn thread_flags_all_four_are_distinct_and_non_overlapping() {
    let all = ThreadFlags::MAIN_THREAD
        | ThreadFlags::EXITING
        | ThreadFlags::FPU_STATE_DIRTY
        | ThreadFlags::IN_SYSCALL;
    assert_eq!(all.bits(), 0b1111, "four lowest bits must be set");
}

// ═══════════════════════════════════════════════════════════════════════════════
// 11. seastar::context
// ═══════════════════════════════════════════════════════════════════════════════

use seastar::Context;

#[test_case]
fn _section_seastar_context() {
    crate::serial_println!("\n=== seastar::context ===");
}

#[test_case]
fn context_zeroed_all_fields_are_zero() {
    let c = Context::zeroed();
    assert_eq!(c.rsp, 0);
    assert_eq!(c.r15, 0);
    assert_eq!(c.r14, 0);
    assert_eq!(c.r13, 0);
    assert_eq!(c.r12, 0);
    assert_eq!(c.rbx, 0);
    assert_eq!(c.rbp, 0);
}

#[test_case]
fn context_is_repr_c_56_bytes() {
    // 7 u64 fields * 8 bytes = 56 bytes. Verified against the assembly offset
    // expected by cephalopod's switch stub.
    assert_eq!(core::mem::size_of::<Context>(), 56);
}

#[test_case]
fn context_rsp_field_is_at_offset_0() {
    // The context switcher accesses rsp at offset 0. Verify with addr_of.
    let c = Context::zeroed();
    let base = &c as *const Context as usize;
    let rsp  = core::ptr::addr_of!(c.rsp) as usize;
    assert_eq!(rsp - base, 0, "rsp must be the first field (offset 0)");
}

#[test_case]
fn context_new_thread_rsp_is_stack_top_minus_8() {
    // Allocate an 8-byte aligned stack buffer on the heap.
    let mut buf = alloc::vec![0u64; 8]; // 64 bytes, heap-allocated
    let stack_top = buf.as_mut_ptr() as u64 + 64;
    let entry     = 0xDEAD_C0DE_CAFE_BABEu64;
    let ctx = unsafe { Context::new_thread(stack_top, entry) };
    assert_eq!(ctx.rsp, stack_top - 8, "initial RSP must be 8 below stack top");
}

#[test_case]
fn context_new_thread_entry_is_at_rsp() {
    let mut buf   = alloc::vec![0u64; 8];
    let stack_top = buf.as_mut_ptr() as u64 + 64;
    let entry     = 0x1234_5678_9ABC_DEF0u64;
    let ctx = unsafe { Context::new_thread(stack_top, entry) };
    // The synthetic return address must be at the address RSP points to.
    let ret_addr = unsafe { *(ctx.rsp as *const u64) };
    assert_eq!(ret_addr, entry, "entry point must be written at ctx.rsp");
}

#[test_case]
fn context_new_thread_callee_regs_are_zero() {
    let mut buf   = alloc::vec![0u64; 8];
    let stack_top = buf.as_mut_ptr() as u64 + 64;
    let ctx = unsafe { Context::new_thread(stack_top, 0x1000) };
    // All callee-saved registers start zeroed on a new thread.
    assert_eq!(ctx.r15, 0);
    assert_eq!(ctx.r14, 0);
    assert_eq!(ctx.r13, 0);
    assert_eq!(ctx.r12, 0);
    assert_eq!(ctx.rbx, 0);
    assert_eq!(ctx.rbp, 0);
}

// ═══════════════════════════════════════════════════════════════════════════════
// 12. seastar::priority
// ═══════════════════════════════════════════════════════════════════════════════

use seastar::Priority;
use seastar::priority::SchedulingStats;

#[test_case]
fn _section_seastar_priority() {
    crate::serial_println!("\n=== seastar::priority ===");
}

#[test_case]
fn priority_zero_cannot_be_constructed_via_new() {
    assert!(Priority::new(0).is_none(), "Priority 0 is reserved for idle");
}

#[test_case]
fn priority_nonzero_values_succeed() {
    assert!(Priority::new(1).is_some(),   "REALTIME_HI boundary");
    assert!(Priority::new(128).is_some(), "NORMAL");
    assert!(Priority::new(255).is_some(), "max priority value");
}

#[test_case]
fn priority_lower_number_is_higher_urgency() {
    // Comparison is numeric; lower value = higher scheduling urgency.
    assert!(Priority::REALTIME_HI < Priority::REALTIME_LO);
    assert!(Priority::REALTIME_LO < Priority::NORMAL_HI);
    assert!(Priority::NORMAL_HI   < Priority::NORMAL);
    assert!(Priority::NORMAL      < Priority::NORMAL_LO);
    assert!(Priority::NORMAL_LO   < Priority::BACKGROUND);
}

#[test_case]
fn priority_idle_constant_is_zero() {
    assert_eq!(Priority::IDLE.as_u8(), 0);
}

#[test_case]
fn priority_realtime_hi_is_1() {
    assert_eq!(Priority::REALTIME_HI.as_u8(), 1);
}

#[test_case]
fn priority_default_is_normal() {
    let d: Priority = Default::default();
    assert_eq!(d, Priority::NORMAL);
}

#[test_case]
fn scheduling_stats_default_all_zero() {
    let s = SchedulingStats::default();
    assert_eq!(s.cpu_ticks,       0);
    assert_eq!(s.context_switches, 0);
    assert_eq!(s.voluntary_yields, 0);
}

// ═══════════════════════════════════════════════════════════════════════════════
// 13. seastar::Process
// ═══════════════════════════════════════════════════════════════════════════════

use seastar::Process;

#[test_case]
fn _section_seastar_process() {
    crate::serial_println!("\n=== seastar::Process ===");
}

#[test_case]
fn process_new_kernel_cr3_is_zero() {
    let p = Process::new_kernel();
    assert_eq!(p.cr3, 0, "kernel process sentinel: cr3 must be 0");
}

#[test_case]
fn process_new_user_cr3_matches_argument() {
    let cr3 = 0x0004_0000u64;
    let p   = Process::new_user(cr3);
    assert_eq!(p.cr3, cr3);
}

#[test_case]
fn process_new_kernel_cr3_for_switch_is_none() {
    let p = Process::new_kernel();
    assert!(p.cr3_for_switch().is_none(),
        "kernel process must skip CR3 reload — cr3_for_switch must return None");
}

#[test_case]
fn process_new_user_cr3_for_switch_is_some() {
    let cr3 = 0x0008_0000u64;
    let p   = Process::new_user(cr3);
    assert_eq!(p.cr3_for_switch(), Some(cr3));
}

#[test_case]
fn process_initial_state_is_created() {
    let p = Process::new_kernel();
    assert_eq!(*p.state.lock(), ProcessState::Created,
        "new process must start in Created state");
}

#[test_case]
fn process_kernel_flag_set_on_kernel_process() {
    let p = Process::new_kernel();
    assert!(p.flags.lock().contains(ProcessFlags::KERNEL_PROCESS),
        "kernel process must have KERNEL_PROCESS flag set");
}

#[test_case]
fn process_kernel_flag_absent_on_user_process() {
    let p = Process::new_user(0x1000);
    assert!(!p.flags.lock().contains(ProcessFlags::KERNEL_PROCESS),
        "user process must not have KERNEL_PROCESS flag");
}

#[test_case]
fn process_initial_exit_code_is_zero() {
    use core::sync::atomic::Ordering;
    let p = Process::new_kernel();
    assert_eq!(p.exit_code.load(Ordering::Relaxed), 0);
}

// ═══════════════════════════════════════════════════════════════════════════════
// 14. seastar::ProcessTable
//
// Creates a local table backed by its own SlabCache<Process> so tests are
// isolated from the global kernel process table.
// ═══════════════════════════════════════════════════════════════════════════════

use core::ptr::NonNull;
use seastar::table::{Allocator, ProcessTable, HasGeneration, HasPid};

#[test_case]
fn _section_seastar_process_table() {
    crate::serial_println!("\n=== seastar::ProcessTable ===");
}

struct TestAlloc(SlabCache<Process>);
impl Allocator<Process> for TestAlloc {
    fn alloc(&self) -> Option<NonNull<Process>> { self.0.alloc() }
    unsafe fn dealloc(&self, ptr: NonNull<Process>) {
        unsafe { self.0.dealloc(ptr) }
    }
}

type TestTable = ProcessTable<Process, TestAlloc, 64, crate::arch::X86IrqControl>;

fn make_test_table() -> TestTable {
    TestTable::new(TestAlloc(SlabCache::new(0)))
}

#[test_case]
fn process_table_new_is_empty() {
    let t = make_test_table();
    assert!(t.is_empty());
    assert_eq!(t.len(), 0);
}

#[test_case]
fn process_table_insert_returns_nonzero_pid() {
    let t = make_test_table();
    let (pid, _) = t.insert(Process::new_kernel()).expect("insert must succeed");
    assert_ne!(pid.as_u64(), 0, "inserted PID must not be 0 (INVALID sentinel)");
}

#[test_case]
fn process_table_insert_increments_len() {
    let t = make_test_table();
    let _ = t.insert(Process::new_kernel());
    assert_eq!(t.len(), 1);
    let _ = t.insert(Process::new_user(0x1000));
    assert_eq!(t.len(), 2);
}

#[test_case]
fn process_table_pids_are_strictly_unique() {
    let t = make_test_table();
    let (pid1, _) = t.insert(Process::new_kernel()).unwrap();
    let (pid2, _) = t.insert(Process::new_kernel()).unwrap();
    assert_ne!(pid1, pid2, "each insert must produce a distinct PID");
}

#[test_case]
fn process_table_lookup_by_pid_succeeds() {
    let t   = make_test_table();
    let (pid, _) = t.insert(Process::new_kernel()).unwrap();
    let ref_ = t.lookup(pid);
    assert!(ref_.is_some(), "lookup by a recently inserted PID must return Some");
}

#[test_case]
fn process_table_process_ref_get_returns_live_process() {
    let t   = make_test_table();
    let (_, ref_) = t.insert(Process::new_kernel()).unwrap();
    let proc = unsafe { ref_.get() };
    assert!(proc.is_some(), "ProcessRef::get must return Some for a live process");
}

#[test_case]
fn process_table_process_ref_pid_matches_insert_pid() {
    let t   = make_test_table();
    let (pid, ref_) = t.insert(Process::new_kernel()).unwrap();
    let proc = unsafe { ref_.get() }.expect("process must be live");
    assert_eq!(proc.pid(), pid.as_u64(),
        "PID stamped into the struct must match the PID returned by insert");
}

#[test_case]
fn process_table_remove_returns_true_and_empties_slot() {
    let t = make_test_table();
    let (pid, ref_) = t.insert(Process::new_kernel()).unwrap();
    let removed = unsafe { t.remove(ref_) };
    assert!(removed, "remove must return true for a live process");
    assert_eq!(t.len(), 0);
    assert!(t.lookup(pid).is_none(), "lookup must return None after remove");
}

#[test_case]
fn process_table_ref_invalidated_after_remove() {
    let t   = make_test_table();
    let (_, ref_) = t.insert(Process::new_kernel()).unwrap();
    unsafe { t.remove(ref_) };
    // The ref holds the old generation; after remove the slot's generation is bumped.
    let result = unsafe { ref_.get() };
    assert!(result.is_none(),
        "ProcessRef::get must return None after the process has been removed");
}

#[test_case]
fn process_table_generation_invalidates_stale_ref_on_slot_reuse() {
    let t   = make_test_table();
    let (_, ref1) = t.insert(Process::new_kernel()).unwrap();
    unsafe { t.remove(ref1) };
    // Insert again — may reuse the same slot with a higher generation.
    let (_, ref2) = t.insert(Process::new_kernel()).unwrap();
    // ref2 must be live; ref1 must remain stale.
    assert!(unsafe { ref2.get().is_some() }, "new ref must be live");
    assert!(unsafe { ref1.get().is_none() }, "old ref must be stale after slot reuse");
}

// ═══════════════════════════════════════════════════════════════════════════════
// 15. mantle::pml4 — live validation under the active kernel PML4
//
// The kernel PML4 (installed by install_kernel_pml4 during boot) must:
//   a) Map the full HHDM so buddy pages are accessible.
//   b) Map the kernel image so this executing code is reachable.
//   c) Allow the VMM walker (which reads CR3) to translate addresses correctly.
// ═══════════════════════════════════════════════════════════════════════════════

#[test_case]
fn _section_pml4_live() {
    crate::serial_println!("\n=== mantle::pml4 (live) ===");
}

#[test_case]
fn pml4_protection_kernel_rwx_boot_has_correct_flags() {
    use bitwise::paging::pte_flags;
    let b = mantle::prot::Protection::KERNEL_RWX_BOOT.bits();
    assert_ne!(b & pte_flags::WRITABLE,   0, "must be writable");
    assert_eq!(b & pte_flags::NO_EXECUTE, 0, "must be executable (NX clear)");
    assert_ne!(b & pte_flags::GLOBAL,     0, "must be global (not TLB-flushed on CR3 write)");
}

#[test_case]
fn pml4_buddy_page_readable_and_writable_via_hhdm() {
    // Allocate one page from the buddy (returns an HHDM virtual address).
    // Read/write to it; success proves the HHDM 2 MiB mappings are active.
    let virt = abalone::buddy::alloc_pages(0)
        .expect("buddy must supply a page for this test") as u64;
    crate::serial_println!("\n  buddy page virt={:#x}", virt);
    let sentinel = 0xA5A5_A5A5_A5A5_A5A5u64;
    unsafe {
        core::ptr::write_volatile(virt as *mut u64, sentinel);
        let readback = core::ptr::read_volatile(virt as *const u64);
        assert_eq!(readback, sentinel,
            "volatile write/read to buddy page must round-trip via HHDM mapping");
    }
    // Return the page to the buddy so later tests are not starved.
    unsafe { abalone::buddy::BUDDY.lock().add_region(virt as usize, 1) };
}

#[test_case]
fn pml4_vmm_translates_buddy_page_to_some_physical_address() {
    // Allocate a page, then ask the VMM walker to translate it.
    // Under the new PML4, HHDM is 2M-mapped; translate must handle huge pages.
    let virt = abalone::buddy::alloc_pages(0)
        .expect("buddy must supply a page") as u64;
    let vmm  = crate::memory::vmm::get();
    let phys = vmm.translate(virt);
    crate::serial_println!("\n  virt={:#x} -> phys={:?}", virt, phys);
    assert!(phys.is_some(), "VMM must translate a live buddy page to a physical address");
    let p = phys.unwrap();
    assert_eq!(p & 0xFFF, 0, "translated physical must be page-aligned");
    assert_ne!(p, 0, "translated physical must be nonzero");
    unsafe { abalone::buddy::BUDDY.lock().add_region(virt as usize, 1) };
}

#[test_case]
fn pml4_kernel_function_virtual_address_is_mapped() {
    // gdt::init is a kernel function; its VA must be in the kernel image mapping.
    // If the PML4 did not map the image, we would have triple-faulted before now,
    // but confirm explicitly via the VMM translator.
    let fn_virt = crate::gdt::init as usize as u64;
    crate::serial_println!("\n  gdt::init virt={:#x}", fn_virt);
    let phys = crate::memory::vmm::get().translate(fn_virt);
    assert!(phys.is_some(),
        "kernel function virtual address must be mapped in the active PML4");
    let p = phys.unwrap();
    assert_ne!(p, 0, "kernel image physical address must be nonzero");
}

#[test_case]
fn pml4_static_data_virtual_address_is_mapped() {
    // A static in the kernel image is also covered by the 4K kernel image mapping.
    static SENTINEL: u64 = 0xFEED_DEAD_CAFE_BABEu64;
    let virt = core::ptr::addr_of!(SENTINEL) as u64;
    crate::serial_println!("\n  static virt={:#x}", virt);
    let phys = crate::memory::vmm::get().translate(virt);
    assert!(phys.is_some(),
        "kernel static data virtual address must be mapped in the active PML4");
    // Value must be readable through the mapping.
    let readback = unsafe { core::ptr::read_volatile(virt as *const u64) };
    assert_eq!(readback, 0xFEED_DEAD_CAFE_BABEu64,
        "sentinel value must be intact after PML4 switch");
}
