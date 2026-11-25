//! ARM-state veneers keep privileged CPSR instructions out of Thumb-1 code.
struct BootCriticalSection;
critical_section::set_impl!(BootCriticalSection);
unsafe impl critical_section::Impl for BootCriticalSection {
    unsafe fn acquire() -> u32 {
        unsafe { mt2502_hal::cpu::acquire_irq_mask() }
    }
    unsafe fn release(saved: u32) {
        unsafe { mt2502_hal::cpu::restore_irq_mask(saved) }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __sync_synchronize() {
    mt2502_hal::cpu::compiler_barrier();
}
