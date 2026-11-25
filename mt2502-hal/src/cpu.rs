//! ARM-state entry points usable from both ARM and Thumb-1 Rust code.
core::arch::global_asm!(
    ".section .text.mt2502_cpu,\"ax\",%progbits",
    ".arm",
    ".balign 4",
    ".global mt2502_irq_acquire",
    ".type mt2502_irq_acquire,%function",
    "mt2502_irq_acquire:",
    "mrs r0,cpsr",
    "orr r1,r0,#0xc0",
    "msr cpsr_c,r1",
    "bx lr",
    ".global mt2502_irq_restore",
    ".type mt2502_irq_restore,%function",
    "mt2502_irq_restore:",
    "msr cpsr_c,r0",
    "bx lr",
    ".global mt2502_irq_enable",
    ".type mt2502_irq_enable,%function",
    "mt2502_irq_enable:",
    "mrs r0,cpsr",
    "bic r0,r0,#0x80",
    "msr cpsr_c,r0",
    "bx lr",
    ".global mt2502_cpsr",
    ".type mt2502_cpsr,%function",
    "mt2502_cpsr:",
    "mrs r0,cpsr",
    "bx lr",
);
unsafe extern "C" {
    fn mt2502_irq_acquire() -> u32;
    fn mt2502_irq_restore(saved: u32);
    fn mt2502_irq_enable();
    fn mt2502_cpsr() -> u32;
}
/// # Safety
/// Pair with restore_irq_mask on the same CPU, respecting critical-section nesting.
pub unsafe fn acquire_irq_mask() -> u32 {
    unsafe { mt2502_irq_acquire() }
}
/// # Safety
/// Restore only a state obtained by the matching acquire on the same CPU.
pub unsafe fn restore_irq_mask(saved: u32) {
    unsafe { mt2502_irq_restore(saved) }
}
pub fn enable_irq() {
    unsafe { mt2502_irq_enable() }
}
pub fn status() -> u32 {
    unsafe { mt2502_cpsr() }
}
pub fn compiler_barrier() {
    unsafe {
        core::arch::asm!("", options(nostack, preserves_flags));
    }
}
