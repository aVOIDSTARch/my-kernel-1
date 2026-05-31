// v0.0.3
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
// Slab tests run after heap::init, so the buddy allocator is seeded and ready.
// SlabCache::new(order) takes a buddy order for slab backing pages.
// SlabCache::alloc() returns Option<NonNull<T>>; dealloc(NonNull<T>) is unsafe.

use abalone::slab::SlabCache;

#[test_case]
fn slab_alloc_returns_valid_pointer() {
    let cache: SlabCache<u64> = SlabCache::new(0);
    let ptr = cache.alloc().expect("SlabCache::alloc must return Some");
    unsafe { ptr.as_ptr().write(0xCAFE_BABE_DEAD_BEEF) };
    assert_eq!(unsafe { ptr.as_ptr().read() }, 0xCAFE_BABE_DEAD_BEEF);
    unsafe { cache.dealloc(ptr) };
}

#[test_case]
fn slab_two_allocs_do_not_overlap() {
    let cache: SlabCache<[u8; 32]> = SlabCache::new(0);
    let p0 = cache.alloc().expect("first slab alloc");
    let p1 = cache.alloc().expect("second slab alloc");
    let diff = (p0.as_ptr() as isize - p1.as_ptr() as isize).unsigned_abs();
    assert!(diff >= 32, "slab allocations overlap");
    unsafe {
        cache.dealloc(p0);
        cache.dealloc(p1);
    }
}

#[test_case]
fn slab_dealloc_and_realloc_succeeds() {
    let cache: SlabCache<u32> = SlabCache::new(0);
    let p0 = cache.alloc().expect("first slab alloc");
    unsafe { cache.dealloc(p0) };
    let p1 = cache.alloc().expect("alloc after dealloc must succeed");
    unsafe { cache.dealloc(p1) };
}
