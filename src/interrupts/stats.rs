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
