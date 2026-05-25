use core::panic::PanicInfo;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    // In a real kernel this would write to serial/VGA.
    // For now, halt all further execution unconditionally.
    loop {
        x86_64::instructions::hlt();
    }
}
