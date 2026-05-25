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
