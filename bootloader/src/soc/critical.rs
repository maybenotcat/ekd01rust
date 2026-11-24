//! ARM-state veneers keep privileged CPSR instructions out of Thumb-1 code.
struct BootCriticalSection;
critical_section::set_impl!(BootCriticalSection);
unsafe extern "C" {
    fn boot_acquire_irq_mask() -> u32;
    fn boot_restore_irq_mask(saved: u32);
}
unsafe impl critical_section::Impl for BootCriticalSection {
    unsafe fn acquire() -> u32 {
        unsafe { boot_acquire_irq_mask() }
    }
    unsafe fn release(saved: u32) {
        unsafe { boot_restore_irq_mask(saved) }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __sync_synchronize() {
    unsafe {
        core::arch::asm!("", options(nostack, preserves_flags));
    }
}
