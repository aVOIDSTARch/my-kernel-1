#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(custom_test_frameworks)]
#![test_runner(crate::testing::test_runner)]
#![reexport_test_harness_main = "test_main"]

mod gdt;
mod interrupts;
mod memory;
mod panic;
mod serial;
mod testing;
mod vga_buffer;

use limine::request::{ExecutableAddressRequest, HhdmRequest, MemmapRequest};
use limine::{BaseRevision, RequestsEndMarker, RequestsStartMarker};

// ── Limine Protocol Anchors ───────────────────────────────────────────────
// Layout required for base revision 2+:
//   .limine_requests_start  → RequestsStartMarker (+ BaseRevision)
//   .limine_requests        → actual requests
//   .limine_requests_end    → RequestsEndMarker

#[used]
#[unsafe(link_section = ".limine_requests_start")]
static REQUESTS_START: RequestsStartMarker = RequestsStartMarker::new();

// Request base revision 2 — the highest revision Limine v8 supports.
// BaseRevision::new() requests revision 6 which Limine v8 does not support.
#[used]
#[unsafe(link_section = ".limine_requests_start")]
static BASE_REVISION: BaseRevision = BaseRevision::with_revision(2);

#[used]
#[unsafe(link_section = ".limine_requests")]
static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
static MEMORY_MAP_REQUEST: MemmapRequest = MemmapRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
static KERNEL_ADDRESS_REQUEST: ExecutableAddressRequest = ExecutableAddressRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests_end")]
static REQUESTS_END: RequestsEndMarker = RequestsEndMarker::new();

// ── Entry Point ───────────────────────────────────────────────────────────
// State at entry (guaranteed by Limine):
//   - Long mode, ring 0
//   - Interrupts DISABLED (IF clear)
//   - Paging ENABLED (Limine's page tables)
//   - No valid GDT for our kernel yet
//   - No IDT loaded
//   - Stack: at least 64 KiB, Limine-provided
//   - SSE/AVX: enabled

#[unsafe(no_mangle)]
pub extern "C" fn kernel_main() -> ! {
    // Serial is safe before anything else — port I/O, no memory mapping needed.
    serial_println!("[kernel] booting...");

    // Consume all Limine responses before GDT/page-table switch may invalidate them.
    let hhdm_offset = HHDM_REQUEST
        .response()
        .expect("Limine: no HHDM response")
        .offset;

    let memory_map = MEMORY_MAP_REQUEST
        .response()
        .expect("Limine: no memory map response");

    let kernel_addr = KERNEL_ADDRESS_REQUEST
        .response()
        .expect("Limine: no kernel address response");

    let kernel_phys_start = kernel_addr.physical_base;

    unsafe extern "C" {
        static __kernel_start: u8;
        static __kernel_end: u8;
    }
    let kernel_size = unsafe {
        (&__kernel_end as *const u8 as u64)
            .saturating_sub(&__kernel_start as *const u8 as u64)
    };
    let kernel_phys_end = kernel_phys_start + kernel_size;

    serial_println!("[kernel] hhdm={:#x}", hhdm_offset);

    // VGA buffer is at phys 0xb8000; only safe via HHDM under Limine v7+.
    vga_buffer::init(hhdm_offset);
    println!("my-kernel booting...");
    println!("kernel: phys {:#x}..{:#x}  hhdm +{:#x}",
        kernel_phys_start, kernel_phys_end, hhdm_offset);

    gdt::init();
    println!("gdt: ok");

    interrupts::init();
    println!("idt: ok");

    memory::pmm::init(
        memory_map.entries(),
        kernel_phys_start,
        kernel_phys_end,
        hhdm_offset,
    );
    println!("pmm: ok");

    memory::pmm::reclaim_bootloader_memory(memory_map.entries());
    println!("pmm: bootloader memory reclaimed");

    x86_64::instructions::interrupts::enable();
    println!("interrupts: enabled");

    #[cfg(test)]
    test_main();

    println!("kernel ready.");
    serial_println!("[kernel] halted.");
    loop {
        x86_64::instructions::hlt();
    }

}
