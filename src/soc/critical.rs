//! Single-core ARMv5 critical sections; nested calls restore the caller's masks.
struct Armv5CriticalSection;
critical_section::set_impl!(Armv5CriticalSection);
unsafe impl critical_section::Impl for Armv5CriticalSection {
    unsafe fn acquire() -> u32 {
        unsafe { mt2502_hal::cpu::acquire_irq_mask() }
    }
    unsafe fn release(saved: u32) {
        unsafe { mt2502_hal::cpu::restore_irq_mask(saved) }
    }
}

/// LLVM's ARMv5 fence fallback. This single-core board uses uncached physical
/// RAM and explicit peripheral completion reads, as in the working C port.
/// There is no Cortex-M DMB or CP15 cache instruction on the MT2502 ARM7 core.
#[unsafe(no_mangle)]
#[inline(never)]
pub extern "C" fn __sync_synchronize() {
    // LLVM lowers even compiler_fence to this ABI helper on this ARMv5 target.
    // Calling that intrinsic here would recurse until the stack corrupts RAM.
    // An opaque asm block has a compiler memory clobber without requesting
    // another fence intrinsic or an unsupported CPU barrier instruction.
    mt2502_hal::cpu::compiler_barrier();
}
