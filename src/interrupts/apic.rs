// v0.0.4
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use spin::Once;

static LAPIC: Once<LocalApic> = Once::new();
static APIC_BASE_VADDR: AtomicU64 = AtomicU64::new(0);
static APIC_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn apic_supported() -> bool {
    // CPUID.1:EDX bit 9 = APIC on-chip.
    let result = core::arch::x86_64::__cpuid(1);
    (result.edx & (1 << 9)) != 0
}

/// Map the LAPIC MMIO region (phys 0xFEE00000) with MMIO_UC before calling this.
/// `vaddr` is the virtual address of that mapping.
///
/// Initializes the LAPIC global, enables SVR software-enable, and sets the
/// spurious vector to 0xFF. Call once after the VMM is up.
pub unsafe fn init_lapic(vaddr: u64) {
    LAPIC.call_once(|| {
        let lapic = unsafe { LocalApic::new(vaddr) };
        unsafe { lapic.init(0xFF) };
        lapic
    });
    APIC_BASE_VADDR.store(vaddr, Ordering::Release);
    APIC_ENABLED.store(true, Ordering::Release);
}

/// Returns a reference to the global LAPIC, or `None` if `init_lapic` has not been called.
pub fn get() -> Option<&'static LocalApic> {
    LAPIC.get()
}

pub struct LocalApic {
    base: u64,
}

// SAFETY: LocalApic is a typed view over MMIO memory at a known fixed physical
// address. Concurrent register accesses are safe because LAPIC registers are
// per-CPU (no bus contention) and all writes are volatile.
unsafe impl Send for LocalApic {}
unsafe impl Sync for LocalApic {}

impl LocalApic {
    /// Safety: `base_vaddr` must be a valid, uncached virtual mapping of the
    /// LAPIC MMIO region (physical 0xFEE00000, size 0x1000).
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

    /// Enable the LAPIC and configure the spurious interrupt vector.
    ///
    /// `spurious_vector` must be 0xF0-0xFF; use 0xFF conventionally.
    pub unsafe fn init(&self, spurious_vector: u8) {
        // Spurious Interrupt Vector Register (SVR, offset 0xF0).
        // Bit 8 = APIC software enable.
        let svr = (spurious_vector as u32) | (1 << 8);
        unsafe { self.write(0xF0, svr); }
    }

    pub unsafe fn end_of_interrupt(&self) {
        unsafe { self.write(0xB0, 0); }
    }

    pub unsafe fn send_ipi(&self, dest_apic_id: u8, vector: u8) {
        unsafe {
            // ICR high (offset 0x310): destination APIC ID.
            self.write(0x310, (dest_apic_id as u32) << 24);
            // ICR low (offset 0x300): write triggers the send.
            self.write(0x300, vector as u32 | (1 << 14)); // Assert, fixed delivery.
        }
    }
}

/// Mask all 8259 PIC lines. Call before switching fully to APIC interrupt delivery.
/// PIC IDT entries remain to handle any residual spurious PIC pulses.
pub unsafe fn disable_pic() {
    use x86_64::instructions::port::Port;
    unsafe {
        Port::<u8>::new(0xA1).write(0xFF); // mask slave
        Port::<u8>::new(0x21).write(0xFF); // mask master
    }
}
