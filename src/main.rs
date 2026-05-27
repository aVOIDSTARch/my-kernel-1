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
use limine::BaseRevision;

// ── Limine Protocol Anchors ───────────────────────────────────────────────
// All must be #[used] or the compiler eliminates them as dead statics.
// All must be in .limine_requests or Limine will not find them.

#[used]
#[unsafe(link_section = ".limine_requests")]
static BASE_REVISION: BaseRevision = BaseRevision::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
static MEMORY_MAP_REQUEST: MemmapRequest = MemmapRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
static KERNEL_ADDRESS_REQUEST: ExecutableAddressRequest = ExecutableAddressRequest::new();

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
    serial_println!("[kernel] booting...");
    println!("my-kernel booting...");

    assert!(
        BASE_REVISION.is_supported(),
        "Limine base revision not supported"
    );

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
    loop {
        x86_64::instructions::hlt();
    }
}
