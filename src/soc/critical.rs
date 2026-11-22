//! Single-core ARMv5 critical sections; nested calls restore the caller's masks.
struct Armv5CriticalSection;
critical_section::set_impl!(Armv5CriticalSection);
unsafe impl critical_section::Impl for Armv5CriticalSection {
    unsafe fn acquire() -> u32 {
        let saved: u32;
        let masked: u32;
        unsafe {
            core::arch::asm!("mrs {saved}, cpsr", "orr {masked}, {saved}, #0xc0", "msr cpsr_c, {masked}",
            saved = out(reg) saved, masked = out(reg) masked, options(nostack));
        }
        let _ = masked;
        saved
    }
    unsafe fn release(saved: u32) {
        unsafe {
            core::arch::asm!("msr cpsr_c, {saved}", saved = in(reg) saved, options(nostack));
        }
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
    unsafe {
        core::arch::asm!("", options(nostack, preserves_flags));
    }
}
