#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

mod gdt;
mod interrupts;
mod memory;
mod panic;
mod serial;
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

    // ── Step 1: Consume all Limine responses immediately.
    // After GDT/page table switch, Limine's mappings may be gone.

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

    // Derive kernel physical end from linker-exported symbols.
    unsafe extern "C" {
        static __kernel_start: u8;
        static __kernel_end: u8;
    }
    let kernel_size = unsafe {
        (&__kernel_end as *const u8 as u64)
            .saturating_sub(&__kernel_start as *const u8 as u64)
    };
    let kernel_phys_end = kernel_phys_start + kernel_size;

    // ── Step 2: Load our GDT and TSS.
    // Limine's GDT is now discarded. All segment registers and the TSS
    // point into our static structures.
    gdt::init();

    // ── Step 3: Load IDT and initialise PIC.
    // The PIC is programmed with vector offsets 0x20/0x28 and all lines
    // are unmasked. Interrupts are still disabled — IF is still clear.
    interrupts::init();

    // ── Step 4: Initialise physical memory manager.
    // Only USABLE pages enter the free pool at this stage.
    // BOOTLOADER_RECLAIMABLE pages are still in use by Limine's responses.
    // let entries: alloc_entries_vec(memory_map.entries());
    // NOTE: if no alloc available, iterate directly:
    memory::pmm::init(
        memory_map.entries(),
        kernel_phys_start,
        kernel_phys_end,
        hhdm_offset,
    );

    // ── Step 5: Reclaim bootloader memory.
    // We have copied everything we need out of Limine's response structures.
    memory::pmm::reclaim_bootloader_memory(memory_map.entries());

    // ── Step 6: Enable interrupts.
    // The PIC will now begin delivering timer ticks and other IRQs.
    x86_64::instructions::interrupts::enable();

    // ── Kernel is operational. Halt loop.
    loop {
        x86_64::instructions::hlt();
    }
}
