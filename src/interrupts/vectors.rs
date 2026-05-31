// v0.0.3
// Every vector number lives here. No other file uses raw numeric literals
// for vectors — they import from this module.

pub const EXCEPTION_DIVIDE_ERROR:        u8 = 0x00;
pub const EXCEPTION_DEBUG:               u8 = 0x01;
pub const EXCEPTION_NMI:                 u8 = 0x02;
pub const EXCEPTION_BREAKPOINT:          u8 = 0x03;
pub const EXCEPTION_OVERFLOW:            u8 = 0x04;
pub const EXCEPTION_BOUND_RANGE:         u8 = 0x05;
pub const EXCEPTION_INVALID_OPCODE:      u8 = 0x06;
pub const EXCEPTION_DEVICE_NOT_AVAIL:    u8 = 0x07;
pub const EXCEPTION_DOUBLE_FAULT:        u8 = 0x08;
pub const EXCEPTION_INVALID_TSS:         u8 = 0x0A;
pub const EXCEPTION_SEGMENT_NOT_PRESENT: u8 = 0x0B;
pub const EXCEPTION_STACK_SEGMENT:       u8 = 0x0C;
pub const EXCEPTION_GENERAL_PROTECTION:  u8 = 0x0D;
pub const EXCEPTION_PAGE_FAULT:          u8 = 0x0E;
pub const EXCEPTION_X87_FP:              u8 = 0x10;
pub const EXCEPTION_ALIGNMENT_CHECK:     u8 = 0x11;
pub const EXCEPTION_MACHINE_CHECK:       u8 = 0x12;
pub const EXCEPTION_SIMD_FP:             u8 = 0x13;
pub const EXCEPTION_VIRTUALIZATION:      u8 = 0x14;

pub const PIC_IRQ_TIMER:          u8 = 0x20;
pub const PIC_IRQ_KEYBOARD:       u8 = 0x21;
pub const PIC_IRQ_SPURIOUS_MASTER:u8 = 0x27;
pub const PIC_IRQ_RTC:            u8 = 0x28;
pub const PIC_IRQ_MOUSE:          u8 = 0x2C;
pub const PIC_IRQ_ATA_PRIMARY:    u8 = 0x2E;
pub const PIC_IRQ_ATA_SECONDARY:  u8 = 0x2F;

pub const APIC_SPURIOUS: u8 = 0xFF;

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptVector {
    Timer          = PIC_IRQ_TIMER,
    Keyboard       = PIC_IRQ_KEYBOARD,
    SpuriousMaster = PIC_IRQ_SPURIOUS_MASTER,
    Rtc            = PIC_IRQ_RTC,
    Mouse          = PIC_IRQ_MOUSE,
    AtaPrimary     = PIC_IRQ_ATA_PRIMARY,
    AtaSecondary   = PIC_IRQ_ATA_SECONDARY,
}

impl InterruptVector {
    #[inline(always)]
    pub fn as_u8(self) -> u8 { self as u8 }

    #[inline(always)]
    pub fn as_usize(self) -> usize { self as usize }

    pub fn is_spurious(self) -> bool {
        matches!(self, Self::SpuriousMaster | Self::AtaSecondary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn pic_irq_offsets_match_8259_remapping() {
        // PIC1 remapped to 0x20, PIC2 to 0x28.
        assert_eq!(PIC_IRQ_TIMER,           0x20);
        assert_eq!(PIC_IRQ_KEYBOARD,        0x21);
        assert_eq!(PIC_IRQ_SPURIOUS_MASTER, 0x27);
        assert_eq!(PIC_IRQ_RTC,             0x28);
        assert_eq!(PIC_IRQ_MOUSE,           0x2C);
        assert_eq!(PIC_IRQ_ATA_PRIMARY,     0x2E);
        assert_eq!(PIC_IRQ_ATA_SECONDARY,   0x2F);
    }

    #[test_case]
    fn interrupt_vector_as_u8_matches_constant() {
        assert_eq!(InterruptVector::Timer.as_u8(),          PIC_IRQ_TIMER);
        assert_eq!(InterruptVector::Keyboard.as_u8(),       PIC_IRQ_KEYBOARD);
        assert_eq!(InterruptVector::SpuriousMaster.as_u8(), PIC_IRQ_SPURIOUS_MASTER);
        assert_eq!(InterruptVector::Rtc.as_u8(),            PIC_IRQ_RTC);
        assert_eq!(InterruptVector::Mouse.as_u8(),          PIC_IRQ_MOUSE);
        assert_eq!(InterruptVector::AtaPrimary.as_u8(),     PIC_IRQ_ATA_PRIMARY);
        assert_eq!(InterruptVector::AtaSecondary.as_u8(),   PIC_IRQ_ATA_SECONDARY);
    }

    #[test_case]
    fn as_usize_matches_as_u8_cast() {
        assert_eq!(InterruptVector::Timer.as_usize(), PIC_IRQ_TIMER as usize);
        assert_eq!(InterruptVector::Rtc.as_usize(),   PIC_IRQ_RTC   as usize);
    }

    #[test_case]
    fn is_spurious_only_for_spurious_vectors() {
        assert!( InterruptVector::SpuriousMaster.is_spurious());
        assert!( InterruptVector::AtaSecondary.is_spurious());
        assert!(!InterruptVector::Timer.is_spurious());
        assert!(!InterruptVector::Keyboard.is_spurious());
        assert!(!InterruptVector::Rtc.is_spurious());
        assert!(!InterruptVector::Mouse.is_spurious());
        assert!(!InterruptVector::AtaPrimary.is_spurious());
    }
}
