// v0.0.2
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static APIC_BASE_VADDR: AtomicU64 = AtomicU64::new(0);
static APIC_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn apic_supported() -> bool {
    // CPUID.1:EDX bit 9 = APIC on-chip
    let result = core::arch::x86_64::__cpuid(1);
    (result.edx & (1 << 9)) != 0
}

/// Map the local APIC MMIO region before calling this.
/// vaddr must be an uncached mapping of the physical APIC base (0xFEE00000).
pub unsafe fn set_mapped_vaddr(vaddr: u64) {
    APIC_BASE_VADDR.store(vaddr, Ordering::Release);
}

pub struct LocalApic {
    base: u64,
}

impl LocalApic {
    /// Safety: base must be a valid, uncached virtual mapping of the LAPIC MMIO region.
    pub unsafe fn new(base_vaddr: u64) -> Self {
        Self { base: base_vaddr }
    }

    unsafe fn read(&self, offset: u32) -> u32 {
        let ptr = (self.base + offset as u64) as *const u32;
        unsafe { ptr.read_volatile() }
    }

    unsafe fn write(&self, offset: u32, val: u32) {
        let ptr = (self.base + offset as u64) as *mut u32;
        unsafe { ptr.write_volatile(val); }
    }

    /// Enable the local APIC and set the spurious vector.
    /// spurious_vector must be 0xF0–0xFF; conventionally 0xFF.
    pub unsafe fn init(&self, spurious_vector: u8) {
        // Spurious Interrupt Vector Register (offset 0xF0)
        // Bit 8 = APIC software enable
        let svr = (spurious_vector as u32) | (1 << 8);
        unsafe { self.write(0xF0, svr); }
        APIC_ENABLED.store(true, Ordering::Release);
    }

    pub unsafe fn end_of_interrupt(&self) {
        unsafe { self.write(0xB0, 0); }
    }

    pub unsafe fn send_ipi(&self, dest_apic_id: u8, vector: u8) {
        unsafe {
            // ICR high (offset 0x310): destination
            self.write(0x310, (dest_apic_id as u32) << 24);
            // ICR low (offset 0x300): write triggers send
            self.write(0x300, vector as u32 | (1 << 14)); // Assert, fixed delivery
        }
    }
}

/// Mask all 8259 lines before enabling the APIC.
/// After this, the PIC will no longer deliver IRQs.
/// Existing PIC IDT entries remain to catch residual spurious PIC interrupts.
pub unsafe fn disable_pic() {
    use x86_64::instructions::port::Port;
    let mut master_data: Port<u8> = Port::new(0xA1);
    let mut slave_data:  Port<u8> = Port::new(0x21);
    unsafe {
        master_data.write(0xFF);
        slave_data.write(0xFF);
    }
}
