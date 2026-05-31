// v0.0.3
use core::sync::atomic::{AtomicPtr, Ordering};

// 16 hardware IRQ lines on the 8259 (IRQ0–IRQ15).
static HANDLERS: [AtomicPtr<()>; 16] = {
    const NULL: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());
    [NULL; 16]
};

pub fn register(irq: u8, handler: fn()) -> Result<(), ()> {
    let ptr = handler as *mut ();
    HANDLERS[irq as usize]
        .compare_exchange(
            core::ptr::null_mut(),
            ptr,
            Ordering::Release,
            Ordering::Relaxed,
        )
        .map(|_| ())
        .map_err(|_| ())
}

pub fn unregister(irq: u8) {
    HANDLERS[irq as usize].store(core::ptr::null_mut(), Ordering::Release);
}

/// Called from within an interrupt handler. No locking. No allocation.
/// Safety: caller must be in interrupt context with interrupts disabled.
#[inline(always)]
pub unsafe fn dispatch(irq: u8) {
    let ptr = HANDLERS[irq as usize].load(Ordering::Acquire);
    if !ptr.is_null() {
        let f: fn() = unsafe { core::mem::transmute(ptr) };
        f();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicBool, Ordering};

    // Use IRQ lines 3-6: not wired to any hardware in the kernel's PIC setup.
    // Tests always unregister after use to avoid cross-test interference.

    static FIRED_3: AtomicBool = AtomicBool::new(false);
    fn handler_3() { FIRED_3.store(true, Ordering::Relaxed); }

    #[test_case]
    fn register_and_dispatch_calls_handler() {
        FIRED_3.store(false, Ordering::Relaxed);
        assert!(register(3, handler_3).is_ok());
        unsafe { dispatch(3); }
        assert!(FIRED_3.load(Ordering::Relaxed), "handler was not called");
        unregister(3);
    }

    #[test_case]
    fn dispatch_with_no_handler_does_not_fault() {
        // IRQ4 has no handler; dispatch must silently return.
        unsafe { dispatch(4); }
    }

    #[test_case]
    fn duplicate_register_is_rejected() {
        fn h1() {}
        fn h2() {}
        assert!(register(5, h1).is_ok());
        assert!(register(5, h2).is_err(), "second register must fail");
        unregister(5);
    }

    static FIRED_6: AtomicBool = AtomicBool::new(false);
    fn handler_6() { FIRED_6.store(true, Ordering::Relaxed); }

    #[test_case]
    fn unregister_prevents_dispatch() {
        FIRED_6.store(false, Ordering::Relaxed);
        assert!(register(6, handler_6).is_ok());
        unregister(6);
        unsafe { dispatch(6); }
        assert!(!FIRED_6.load(Ordering::Relaxed), "handler fired after unregister");
    }

    #[test_case]
    fn reregister_after_unregister_succeeds() {
        fn h() {}
        assert!(register(3, h).is_ok());
        unregister(3);
        assert!(register(3, h).is_ok(), "re-register after unregister must succeed");
        unregister(3);
    }
}
