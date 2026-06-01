//! Architecture-specific implementations injected into portable crates.
//!
//! `X86IrqControl` is the kernel's concrete implementation of
//! `pincer::psync::IrqControl`. It is passed as the `I` type parameter
//! to `IrqMutex` and `ProcessTable` wherever IRQ-safe locking is required.

use core::arch::asm;
use pincer::psync::IrqControl;

/// x86_64 interrupt flag save/restore via `pushfq`/`popfq` + `cli`.
///
/// Saves the full RFLAGS (including IF) before disabling interrupts.
/// Restores the exact saved state on drop — if interrupts were already
/// disabled at the call site, they remain disabled after the guard drops.
/// This is the correct behaviour for nested IrqMutex acquisitions.
pub struct X86IrqControl {
    saved_flags: u64,
}

impl IrqControl for X86IrqControl {
    fn save_and_disable() -> Self {
        let flags: u64;
        unsafe {
            asm!(
                "pushfq",
                "pop {0}",
                "cli",
                out(reg) flags,
                options(nomem, preserves_flags),
            );
        }
        Self { saved_flags: flags }
    }

    fn restore(self) {
        unsafe {
            asm!(
                "push {0}",
                "popfq",
                in(reg) self.saved_flags,
                options(nomem),
            );
        }
    }
}
