// v0.0.7
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
pub mod post_stack_state;
#[cfg(test)]
mod tests;
mod timer;
mod writers;

use limine_data::LimineData;
use post_stack_state::PostStackState;

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
    serial_println!("[kernel] booting...");

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

    // ── Step 2.5: PIT timer ───────────────────────────────────────────────
    timer::init();
    serial_println!("[kernel] timer ok");

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

    // ── Step 4.5: LAPIC ───────────────────────────────────────────────────
    if interrupts::apic::apic_supported() {
        const LAPIC_PHYS: u64 = 0xFEE0_0000;
        const LAPIC_SIZE: u64 = 0x1000;
        let lapic_virt = boot.hhdm_offset + LAPIC_PHYS;
        unsafe {
            memory::vmm::get()
                .map_mmio(lapic_virt, LAPIC_PHYS, LAPIC_SIZE,
                          mantle::prot::Protection::MMIO_UC)
                .expect("LAPIC MMIO map failed");
            interrupts::apic::init_lapic(lapic_virt);
        }
        serial_println!("[kernel] lapic ok");
    }

    // ── Step 5: framebuffer ───────────────────────────────────────────────
    if let Some(fb) = boot.framebuffer {
        serial_println!("[kernel] fb: virt={:#x} phys={:#x} size={:#x}",
            fb.virt_addr, fb.phys_addr, fb.byte_size);
        unsafe {
            memory::vmm::get()
                .map_mmio(fb.virt_addr, fb.phys_addr, fb.byte_size,
                          mantle::prot::Protection::MMIO_WC)
                .expect("fb MMIO map failed");
        }
        serial_println!("[kernel] fb mapped");
        writers::framebuffer::init_from_info(fb);
        serial_println!("[kernel] fb init ok");
    }

    // ── Step 6: allocate permanent kernel stack with guard page ──────────
    // Must happen before release() so the new stack pages come from Usable
    // memory, not from the reclaimable pool we are about to free.
    let kstack = unsafe { memory::stack::alloc_kernel_stack(8) };
    serial_println!("[kernel] stack: top={:#x} guard={:#x}",
        kstack.top, kstack.guard_virt);

    // ── Step 7: transfer state across the stack boundary, then switch ─────
    // boot.release() runs from kernel_main_continue on the new stack,
    // so RSP is in Usable memory when reclaimable pages are fed to the buddy.
    post_stack_state::store(PostStackState {
        rsdp_phys: boot.rsdp_phys,
        boot,
    });
    unsafe { memory::stack::switch_stack(kstack.top, kernel_main_continue) }
}

// ── Post-stack-switch entry point ─────────────────────────────────────────────
// Runs on the buddy-allocated kernel stack. The Limine boot stack is
// abandoned and will be reclaimed along with the rest of BootloaderReclaimable.

fn kernel_main_continue() -> ! {
    let PostStackState { rsdp_phys, boot } = post_stack_state::take();

    // RSP is now in Usable memory; no Limine pointer is live.
    // Release all reclaimable pages, including the former boot stack region.
    unsafe { boot.release() };
    serial_println!("[kernel] boot pages released");

    println!("my-kernel booting...");
    println!("heap: ok  vmm: ok");

    x86_64::instructions::interrupts::enable();
    println!("interrupts: enabled");

    if let Some(addr) = rsdp_phys {
        serial_println!("[kernel] rsdp: phys={:#x}", addr);
    }

    #[cfg(test)]
    test_main();

    println!("kernel ready.");
    serial_println!("[kernel] halted.");
    loop {
        x86_64::instructions::hlt();
    }
}
