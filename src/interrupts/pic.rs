use pic8259::ChainedPics;
use spin::Mutex;

pub const PIC_1_OFFSET: u8 = 0x20;
pub const PIC_2_OFFSET: u8 = 0x28;

static PICS: Mutex<ChainedPics> = Mutex::new(unsafe {
    ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET)
});

pub fn init() {
    unsafe { PICS.lock().initialize() };
}

pub unsafe fn end_of_interrupt(vector: u8) {
    unsafe { PICS.lock().notify_end_of_interrupt(vector); }
}

pub fn mask_irq(irq: u8) {
    use x86_64::instructions::port::Port;
    let _pics = PICS.lock();
    let mut port: Port<u8> = if irq < 8 {
        Port::new(0x21)
    } else {
        Port::new(0xA1)
    };
    let shift = if irq < 8 { irq } else { irq - 8 };
    unsafe {
        let current = port.read();
        port.write(current | (1 << shift));
    }
}

pub fn unmask_irq(irq: u8) {
    use x86_64::instructions::port::Port;
    let mut port: Port<u8> = if irq < 8 {
        Port::new(0x21)
    } else {
        Port::new(0xA1)
    };
    let shift = if irq < 8 { irq } else { irq - 8 };
    unsafe {
        let current = port.read();
        port.write(current & !(1 << shift));
    }
}

pub fn read_imr() -> u16 {
    use x86_64::instructions::port::Port;
    let mut master: Port<u8> = Port::new(0x21);
    let mut slave:  Port<u8> = Port::new(0xA1);
    unsafe { (master.read() as u16) | ((slave.read() as u16) << 8) }
}

/// Read In-Service Register to detect spurious IRQs.
/// Returns true if the IRQ line is genuinely in-service (not spurious).
pub fn irq_in_service(irq: u8) -> bool {
    use x86_64::instructions::port::Port;
    // OCW3: read ISR
    let (port_addr, bit) = if irq < 8 {
        (0x20u16, irq)
    } else {
        (0xA0u16, irq - 8)
    };
    let mut port: Port<u8> = Port::new(port_addr);
    unsafe {
        port.write(0x0Bu8); // OCW3: read ISR
        let isr = port.read();
        (isr & (1 << bit)) != 0
    }
}

pub unsafe fn eoi_if_not_spurious_master() {
    // IRQ7 from master: check ISR before sending EOI.
    if irq_in_service(7) {
        unsafe { end_of_interrupt(0x27); }
    }
    // If spurious: no EOI at all.
}

pub unsafe fn eoi_if_not_spurious_slave() {
    // IRQ15 from slave: if spurious, send master EOI only.
    if irq_in_service(15) {
        unsafe { end_of_interrupt(0x2F); }
    } else {
        // Spurious from slave — master cascade line did assert.
        unsafe { end_of_interrupt(0x20); } // master EOI only
    }
}
