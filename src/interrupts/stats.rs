// v0.0.3
use core::sync::atomic::{AtomicU64, Ordering};

static COUNTERS: [AtomicU64; 256] = {
    const ZERO: AtomicU64 = AtomicU64::new(0);
    [ZERO; 256]
};

#[inline(always)]
pub fn record(vector: u8) {
    COUNTERS[vector as usize].fetch_add(1, Ordering::Relaxed);
}

pub fn count(vector: u8) -> u64 {
    COUNTERS[vector as usize].load(Ordering::Acquire)
}

/// Only for test harnesses. Calling while handlers fire produces transient undercounts.
pub unsafe fn reset_all() {
    for c in &COUNTERS {
        c.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Use exception vectors that never fire spontaneously: overflow (4) and
    // bound-range (5). Timer (0x20) and keyboard (0x21) are excluded.
    const V_A: u8 = 0x04;
    const V_B: u8 = 0x05;

    #[test_case]
    fn record_increments_count_by_one() {
        let before = count(V_A);
        record(V_A);
        assert_eq!(count(V_A), before + 1);
    }

    #[test_case]
    fn multiple_records_accumulate() {
        let before = count(V_B);
        record(V_B);
        record(V_B);
        record(V_B);
        assert_eq!(count(V_B), before + 3);
    }

    #[test_case]
    fn reset_all_zeroes_counters() {
        record(V_A);
        record(V_B);
        unsafe { reset_all(); }
        assert_eq!(count(V_A), 0);
        assert_eq!(count(V_B), 0);
    }

    #[test_case]
    fn count_for_unrecorded_vector_starts_at_zero_after_reset() {
        unsafe { reset_all(); }
        // vector 0x06 (invalid opcode) never fires in QEMU test context.
        assert_eq!(count(0x06), 0);
    }
}
