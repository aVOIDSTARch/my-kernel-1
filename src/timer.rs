// v0.0.2
use core::sync::atomic::{AtomicU64, Ordering};

/// PIT oscillator frequency in Hz (standard 14.318 MHz / 12).
const PIT_BASE_HZ: u32 = 1_193_182;

/// Kernel tick rate in Hz. One tick = one millisecond.
pub const TIMER_HZ: u32 = 1_000;

/// Reload value written to PIT channel 0 (PIT_BASE_HZ / TIMER_HZ, rounded).
const PIT_DIVISOR: u16 = (PIT_BASE_HZ / TIMER_HZ) as u16; // 1193 -> ~1000.15 Hz

/// Tick count since init. Incremented once per timer ISR; never wraps in practice
/// (u64 at 1 kHz overflows in ~585 million years).
static TICKS: AtomicU64 = AtomicU64::new(0);

/// Program PIT channel 0 as a 1 kHz rate generator.
///
/// Call before enabling interrupts. Safe to call exactly once.
pub fn init() {
    use x86_64::instructions::port::Port;
    unsafe {
        // Command: channel 0, lo/hi byte access, mode 2 (rate generator), binary.
        Port::<u8>::new(0x43).write(0x34);
        // Reload value, low byte then high byte.
        Port::<u8>::new(0x40).write(PIT_DIVISOR as u8);
        Port::<u8>::new(0x40).write((PIT_DIVISOR >> 8) as u8);
    }
}

/// Increment the tick counter. Called exclusively from the timer ISR (IRQ0).
#[inline(always)]
pub fn tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
}

/// Milliseconds elapsed since `init()` was called.
#[inline]
pub fn uptime_ms() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// Busy-wait for at least `ms` milliseconds.
///
/// Requires interrupts to be enabled; if interrupts are off the timer ISR
/// never fires and this spins forever.
pub fn sleep_ms(ms: u64) {
    let end = uptime_ms().saturating_add(ms);
    while uptime_ms() < end {
        core::hint::spin_loop();
    }
}
