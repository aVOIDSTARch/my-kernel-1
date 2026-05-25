use super::handlers::*;
use super::vectors::*;
use crate::gdt;
use lazy_static::lazy_static;
use x86_64::structures::idt::InterruptDescriptorTable;

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();

        // CPU exceptions
        idt.divide_error.set_handler_fn(divide_error_handler);
        idt.debug.set_handler_fn(debug_handler);
        unsafe {
            idt.non_maskable_interrupt
                .set_handler_fn(nmi_handler)
                .set_stack_index(gdt::NMI_IST_INDEX);
        }
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        idt.overflow.set_handler_fn(overflow_handler);
        idt.bound_range_exceeded.set_handler_fn(bound_range_handler);
        idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
        idt.device_not_available.set_handler_fn(device_not_available_handler);
        unsafe {
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        }
        idt.invalid_tss.set_handler_fn(invalid_tss_handler);
        idt.segment_not_present.set_handler_fn(segment_not_present_handler);
        idt.stack_segment_fault.set_handler_fn(stack_segment_handler);
        idt.general_protection_fault.set_handler_fn(general_protection_handler);
        idt.page_fault.set_handler_fn(page_fault_handler);
        idt.x87_floating_point.set_handler_fn(x87_fp_handler);
        idt.alignment_check.set_handler_fn(alignment_check_handler);
        unsafe {
            idt.machine_check
                .set_handler_fn(machine_check_handler)
                .set_stack_index(gdt::MACHINE_CHECK_IST_INDEX);
        }
        idt.simd_floating_point.set_handler_fn(simd_fp_handler);
        idt.virtualization.set_handler_fn(virtualization_handler);

        // Hardware IRQs
        idt[PIC_IRQ_TIMER].set_handler_fn(timer_handler);
        idt[PIC_IRQ_KEYBOARD].set_handler_fn(keyboard_handler);
        idt[PIC_IRQ_SPURIOUS_MASTER].set_handler_fn(spurious_master_handler);
        idt[PIC_IRQ_RTC].set_handler_fn(rtc_handler);
        idt[PIC_IRQ_MOUSE].set_handler_fn(mouse_handler);
        idt[PIC_IRQ_ATA_PRIMARY].set_handler_fn(ata_primary_handler);
        idt[PIC_IRQ_ATA_SECONDARY].set_handler_fn(ata_secondary_handler);

        idt
    };
}

pub fn init() {
    IDT.load();
}

