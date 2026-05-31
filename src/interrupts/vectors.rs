// v0.0.2
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
