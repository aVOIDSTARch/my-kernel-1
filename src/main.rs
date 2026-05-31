#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(custom_test_frameworks)]
#![test_runner(crate::testing::test_runner)]
#![reexport_test_harness_main = "test_main"]
extern crate alloc;

mod gdt;
mod interrupts;
mod limine_data;
mod memory;
mod panic;
mod testing;
mod writers;

use limine_data::LimineData;

// ── Entry Point ───────────────────────────────────────────────────────────────
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
    // Serial needs no memory mapping — safe before anything else.
    serial_println!("[kernel] booting...");

    // ── Step 1: harvest all Limine data into owned plain values ──────────
    // After this call, no code in this file holds a Limine response pointer.
    // The request statics in limine_data.rs are private and cannot be
    // accessed from here even accidentally.
    let boot = unsafe { LimineData::harvest() };

    serial_println!("[kernel] hhdm={:#x}", boot.hhdm_offset);
    serial_println!("[kernel] phys={:#x}..{:#x}",
        boot.kernel_phys_start, boot.kernel_phys_end);

    if let Some(ref bl) = boot.bootloader_info {
        serial_println!("[kernel] bootloader: {} {}", bl.name_str(), bl.version_str());
    }

    for region in boot.regions() {
        serial_println!("[memmap] {:#x}+{:#x} {:?}",
            region.base, region.length, region.region_type);
    }

    // ── Step 2: GDT and IDT (static structures, no heap required) ────────
    gdt::init();
    serial_println!("[kernel] gdt ok");

    interrupts::init();
    serial_println!("[kernel] idt ok");

    // ── Step 3: heap (buddy seeded from usable regions, TLSF on top) ─────
    memory::heap::init(
        boot.regions(),
        boot.kernel_phys_start,
        boot.kernel_phys_end,
        boot.hhdm_offset,
    );
    serial_println!("[kernel] heap ok");

    // ── Step 4: VMM ───────────────────────────────────────────────────────
    memory::vmm::init(boot.hhdm_offset);
    serial_println!("[kernel] vmm ok");

    // ── Step 5: framebuffer ───────────────────────────────────────────────
    // Limine does NOT include the framebuffer region in its HHDM mapping.
    // Map it explicitly before touching the framebuffer address.
    if let Some(fb) = boot.framebuffer {
        serial_println!("[kernel] fb: virt={:#x} phys={:#x} size={:#x}",
            fb.virt_addr, fb.phys_addr, fb.byte_size);
        unsafe {
            let _ = memory::vmm::get().map_mmio(fb.virt_addr, fb.phys_addr, fb.byte_size);
        }
        serial_println!("[kernel] fb mapped");
        writers::framebuffer::init_from_info(fb);
        serial_println!("[kernel] fb init ok");
    }

    // ── Step 6: release bootloader-reclaimable pages into the buddy ───────
    // Safety: VMM is up, all Limine response data has been consumed into
    // `boot`'s owned fields. No pointer into reclaimable memory is live.
    unsafe { boot.release() };
    serial_println!("[kernel] boot pages released");

    // ── Kernel is fully initialized ───────────────────────────────────────
    println!("my-kernel booting...");
    println!("heap: ok  vmm: ok");

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
