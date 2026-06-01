// v0.0.8
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

    // ── Step 2: GDT and IDT ──────────────────────────────────────────────
    gdt::init();
    serial_println!("[kernel] gdt ok");

    interrupts::init();
    serial_println!("[kernel] idt ok");

    // ── Step 2.5: PIT timer ───────────────────────────────────────────────
    timer::init();
    serial_println!("[kernel] timer ok");

    // ── Step 3: heap ──────────────────────────────────────────────────────
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

    // ── Step 5.5: identify the boot stack reclaimable region ─────────────
    // release() skips the region whose physical range contains RSP (SP guard).
    // Capture that range now so kernel_main_continue can add it to the buddy
    // after we are running on the new Usable-memory stack.
    let boot_stack_region: Option<(u64, usize)> = {
        let current_sp: u64;
        unsafe {
            core::arch::asm!("mov {}, rsp", out(reg) current_sp, options(nostack, nomem));
        }
        let hhdm = boot.hhdm_offset;
        let sp_phys = (current_sp - hhdm) & !0xFFF;
        boot.reclaimable_regions()
            .find(|r| {
                let base = r.aligned_base();
                let end  = r.aligned_end();
                base >= 0x100000 && base < end && sp_phys >= base && sp_phys < end
            })
            .and_then(|r| {
                let base  = r.aligned_base();
                let end   = r.aligned_end();
                let pages = ((end - base) / 4096) as usize;
                if pages > 0 { Some((hhdm + base, pages)) } else { None }
            })
    };

    let rsdp_phys = boot.rsdp_phys;

    // ── Step 6: release reclaimable pages (SP guard skips boot stack) ────
    unsafe { boot.release() };
    serial_println!("[kernel] boot pages released (boot stack region deferred)");

    // ── Step 7: allocate permanent kernel stack with guard page ──────────
    let kstack = unsafe { memory::stack::alloc_kernel_stack(8) };
    serial_println!("[kernel] stack: top={:#x} guard={:#x}",
        kstack.top, kstack.guard_virt);

    // ── Step 8: store state and switch to permanent kernel stack ──────────
    post_stack_state::store(PostStackState {
        rsdp_phys,
        boot_stack_region,
    });
    unsafe { memory::stack::switch_stack(kstack.top, kernel_main_continue) }
}

// ── Post-stack-switch entry point ─────────────────────────────────────────────

fn kernel_main_continue() -> ! {
    let PostStackState { rsdp_phys, boot_stack_region } = post_stack_state::take();

    // RSP is now in Usable memory; the former boot stack pages are safe to free.
    if let Some((virt_base, page_count)) = boot_stack_region {
        let mut buddy = abalone::buddy::BUDDY.lock();
        unsafe { buddy.add_region(virt_base as usize, page_count); }
        serial_println!("[kernel] boot stack pages released: base={:#x} pages={}",
            virt_base, page_count);
    }

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
