// v0.0.6
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn tick_increases_uptime_ms() {
        let before = uptime_ms();
        tick();
        // Timer ISR may also have fired concurrently; after >= before + 1 always holds
        // because tick() adds exactly 1 and the ISR only adds more.
        assert!(uptime_ms() >= before + 1, "tick() must increase uptime by at least 1");
    }

    #[test_case]
    fn uptime_ms_is_non_decreasing() {
        let a = uptime_ms();
        let b = uptime_ms();
        let c = uptime_ms();
        assert!(b >= a, "uptime went backwards between reads");
        assert!(c >= b, "uptime went backwards between reads");
    }

    #[test_case]
    fn sleep_ms_zero_returns_without_spinning() {
        let before = uptime_ms();
        sleep_ms(0);
        // Must complete; after >= before (timer only moves forward).
        assert!(uptime_ms() >= before);
    }

    #[test_case]
    fn sleep_ms_advances_uptime_by_at_least_requested_duration() {
        assert!(
            x86_64::instructions::interrupts::are_enabled(),
            "interrupts must be enabled"
        );

        // Verify ticks are actually advancing before committing to sleep_ms.
        // Wait at most 100 ms (100 ticks) for a single tick to arrive.
        let probe_start = uptime_ms();
        loop {
            if uptime_ms() > probe_start { break; }
            if uptime_ms() > probe_start + 100 {
                panic!("timer ISR is not calling tick() — TICKS frozen at {}", probe_start);
            }
            core::hint::spin_loop();
        }

        let before = uptime_ms();
        sleep_ms(5);
        let after = uptime_ms();
        assert!(
            after >= before + 5,
            "sleep_ms(5) returned too early: before={} after={} elapsed={}",
            before, after, after - before,
        );
    }
}
