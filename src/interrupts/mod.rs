pub mod apic;
pub mod dispatch;
pub mod exceptions;
pub mod handlers;
pub mod pic;
pub mod stats;
pub mod vectors;

pub fn init() {
    exceptions::init(); // Load IDT — must be first
    pic::init();        // Program 8259 — must follow IDT load
    // Interrupts remain disabled. Caller enables with sti.
}
