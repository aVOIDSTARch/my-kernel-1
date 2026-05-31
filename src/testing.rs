// v0.0.2
use x86_64::instructions::port::Port;

// ── QEMU exit device ──────────────────────────────────────────────────────
// Port 0xf4, size 4 — requires `-device isa-debug-exit,iobase=0xf4,iosize=0x04`
// QEMU real exit code = (value << 1) | 1:
//   Success 0x10 → code 33   Failure 0x11 → code 35

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed  = 0x11,
}

pub fn exit_qemu(code: QemuExitCode) -> ! {
    unsafe {
        let mut port: Port<u32> = Port::new(0xf4);
        port.write(code as u32);
    }
    loop {}
}

// ── Testable trait ────────────────────────────────────────────────────────

pub trait Testable {
    fn run(&self);
}

// The impl uses serial macros, so it is test-only.
#[cfg(test)]
impl<T: Fn()> Testable for T {
    fn run(&self) {
        use crate::{serial_print, serial_println};
        serial_print!("{}... ", core::any::type_name::<T>());
        self();
        serial_println!("[ok]");
    }
}

// ── Runner (called by the custom test framework) ──────────────────────────

#[cfg(test)]
pub fn test_runner(tests: &[&dyn Testable]) {
    use crate::serial_println;
    serial_println!("\nrunning {} test{}", tests.len(), if tests.len() == 1 { "" } else { "s" });
    for test in tests {
        test.run();
    }
    serial_println!("all tests passed");
    exit_qemu(QemuExitCode::Success);
}

// ── Panic handler for test mode ───────────────────────────────────────────

#[cfg(test)]
pub fn test_panic_handler(info: &core::panic::PanicInfo) -> ! {
    use crate::serial_println;
    serial_println!("[FAILED]");
    serial_println!("{}", info);
    exit_qemu(QemuExitCode::Failed);
}

// ── Sanity test ───────────────────────────────────────────────────────────

#[test_case]
fn trivial() {
    assert_eq!(1 + 1, 2);
}
