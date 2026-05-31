// v0.0.4
use super::{dispatch, pic, stats, vectors::*};
use lazy_static::lazy_static;
use pc_keyboard::{layouts, HandleControl, PS2Keyboard, ScancodeSet1};
use spin::Mutex;
use x86_64::registers::control::Cr2;
use x86_64::structures::idt::{InterruptStackFrame, PageFaultErrorCode};

// ── CPU Exception Handlers ────────────────────────────────────────────────

pub extern "x86-interrupt" fn divide_error_handler(frame: InterruptStackFrame) {
    stats::record(EXCEPTION_DIVIDE_ERROR);
    panic!("#DE divide error\n{:#?}", frame);
}

pub extern "x86-interrupt" fn debug_handler(_frame: InterruptStackFrame) {
    stats::record(EXCEPTION_DEBUG);
    // Trap — log and return. Do not panic.
}

pub extern "x86-interrupt" fn nmi_handler(frame: InterruptStackFrame) {
    stats::record(EXCEPTION_NMI);
    panic!("NMI — hardware failure\n{:#?}", frame);
}

pub extern "x86-interrupt" fn breakpoint_handler(_frame: InterruptStackFrame) {
    stats::record(EXCEPTION_BREAKPOINT);
    // Trap — must return.
}

pub extern "x86-interrupt" fn overflow_handler(_frame: InterruptStackFrame) {
    stats::record(EXCEPTION_OVERFLOW);
    // Trap — return.
}

pub extern "x86-interrupt" fn bound_range_handler(frame: InterruptStackFrame) {
    stats::record(EXCEPTION_BOUND_RANGE);
    panic!("#BR bound range exceeded\n{:#?}", frame);
}

pub extern "x86-interrupt" fn invalid_opcode_handler(frame: InterruptStackFrame) {
    stats::record(EXCEPTION_INVALID_OPCODE);
    panic!("#UD invalid opcode\n{:#?}", frame);
}

pub extern "x86-interrupt" fn device_not_available_handler(frame: InterruptStackFrame) {
    stats::record(EXCEPTION_DEVICE_NOT_AVAIL);
    panic!("#NM device not available — implement lazy FPU switching\n{:#?}", frame);
}

pub extern "x86-interrupt" fn double_fault_handler(
    frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    stats::record(EXCEPTION_DOUBLE_FAULT);
    panic!("#DF double fault\n{:#?}", frame);
}

pub extern "x86-interrupt" fn invalid_tss_handler(
    frame: InterruptStackFrame,
    error_code: u64,
) {
    stats::record(EXCEPTION_INVALID_TSS);
    panic!("#TS invalid TSS (error={:#x})\n{:#?}", error_code, frame);
}

pub extern "x86-interrupt" fn segment_not_present_handler(
    frame: InterruptStackFrame,
    error_code: u64,
) {
    stats::record(EXCEPTION_SEGMENT_NOT_PRESENT);
    panic!("#NP segment not present (error={:#x})\n{:#?}", error_code, frame);
}

pub extern "x86-interrupt" fn stack_segment_handler(
    frame: InterruptStackFrame,
    error_code: u64,
) {
    stats::record(EXCEPTION_STACK_SEGMENT);
    panic!("#SS stack segment fault (error={:#x})\n{:#?}", error_code, frame);
}

pub extern "x86-interrupt" fn general_protection_handler(
    frame: InterruptStackFrame,
    error_code: u64,
) {
    stats::record(EXCEPTION_GENERAL_PROTECTION);
    panic!("#GP general protection fault (error={:#x})\n{:#?}", error_code, frame);
}

pub extern "x86-interrupt" fn page_fault_handler(
    frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    stats::record(EXCEPTION_PAGE_FAULT);
    // Read CR2 immediately — a subsequent fault would overwrite it.
    let fault_addr = Cr2::read_raw();
    panic!(
        "#PF page fault at {:#x} (error={:?})\n{:#?}",
        fault_addr, error_code, frame
    );
}

pub extern "x86-interrupt" fn x87_fp_handler(frame: InterruptStackFrame) {
    stats::record(EXCEPTION_X87_FP);
    panic!("#MF x87 FP exception\n{:#?}", frame);
}

pub extern "x86-interrupt" fn alignment_check_handler(
    frame: InterruptStackFrame,
    error_code: u64,
) {
    stats::record(EXCEPTION_ALIGNMENT_CHECK);
    panic!("#AC alignment check (error={:#x})\n{:#?}", error_code, frame);
}

pub extern "x86-interrupt" fn machine_check_handler(frame: InterruptStackFrame) -> ! {
    stats::record(EXCEPTION_MACHINE_CHECK);
    panic!("#MC machine check — hardware error\n{:#?}", frame);
}

pub extern "x86-interrupt" fn simd_fp_handler(frame: InterruptStackFrame) {
    stats::record(EXCEPTION_SIMD_FP);
    panic!("#XM SIMD FP exception\n{:#?}", frame);
}

pub extern "x86-interrupt" fn virtualization_handler(frame: InterruptStackFrame) {
    stats::record(EXCEPTION_VIRTUALIZATION);
    panic!("#VE virtualization exception\n{:#?}", frame);
}

// ── Hardware IRQ Handlers ─────────────────────────────────────────────────

pub extern "x86-interrupt" fn timer_handler(_frame: InterruptStackFrame) {
    stats::record(PIC_IRQ_TIMER);
    crate::timer::tick();
    unsafe {
        dispatch::dispatch(0); // IRQ0
        pic::end_of_interrupt(PIC_IRQ_TIMER);
    }
}

pub extern "x86-interrupt" fn apic_spurious_handler(_frame: InterruptStackFrame) {
    stats::record(APIC_SPURIOUS);
    // Spurious LAPIC interrupts must not be acknowledged with an EOI.
}

lazy_static! {
    static ref KEYBOARD: Mutex<PS2Keyboard<layouts::Us104Key, ScancodeSet1>> =
        Mutex::new(PS2Keyboard::new(
            ScancodeSet1::new(),
            layouts::Us104Key,
            HandleControl::Ignore,
        ));
}

pub extern "x86-interrupt" fn keyboard_handler(_frame: InterruptStackFrame) {
    use x86_64::instructions::port::Port;
    stats::record(PIC_IRQ_KEYBOARD);

    // Unconditional read — must happen even if we discard the result.
    let scancode: u8 = unsafe { Port::new(0x60).read() };

    let mut keyboard = KEYBOARD.lock();
    if let Ok(Some(key_event)) = keyboard.add_byte(scancode) {
        if let Some(_key) = keyboard.process_keyevent(key_event) {
            // Deliver to input subsystem when one exists.
        }
    }

    unsafe { pic::end_of_interrupt(PIC_IRQ_KEYBOARD) };
}

pub extern "x86-interrupt" fn spurious_master_handler(_frame: InterruptStackFrame) {
    stats::record(PIC_IRQ_SPURIOUS_MASTER);
    unsafe { pic::eoi_if_not_spurious_master() };
}

pub extern "x86-interrupt" fn rtc_handler(_frame: InterruptStackFrame) {
    use x86_64::instructions::port::Port;
    stats::record(PIC_IRQ_RTC);
    // Must read register C to dismiss the RTC interrupt, or it will not fire again.
    unsafe {
        Port::<u8>::new(0x70).write(0x0C);
        Port::<u8>::new(0x71).read();
        dispatch::dispatch(8); // IRQ8
        pic::end_of_interrupt(PIC_IRQ_RTC);
    }
}

pub extern "x86-interrupt" fn mouse_handler(_frame: InterruptStackFrame) {
    use x86_64::instructions::port::Port;
    stats::record(PIC_IRQ_MOUSE);
    let _data: u8 = unsafe { Port::new(0x60).read() };
    unsafe {
        dispatch::dispatch(12); // IRQ12
        pic::end_of_interrupt(PIC_IRQ_MOUSE);
    }
}

pub extern "x86-interrupt" fn ata_primary_handler(_frame: InterruptStackFrame) {
    stats::record(PIC_IRQ_ATA_PRIMARY);
    unsafe {
        dispatch::dispatch(14); // IRQ14
        pic::end_of_interrupt(PIC_IRQ_ATA_PRIMARY);
    }
}

pub extern "x86-interrupt" fn ata_secondary_handler(_frame: InterruptStackFrame) {
    stats::record(PIC_IRQ_ATA_SECONDARY);
    unsafe { pic::eoi_if_not_spurious_slave() };
}
