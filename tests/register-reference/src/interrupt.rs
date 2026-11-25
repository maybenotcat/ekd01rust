use crate::{mmio::read32, registers::*};
pub fn pending() -> Option<u32> {
    let status = read32(IRQ_STA2);
    if status & IRQ_STA2_INVALID != 0 {
        None
    } else {
        Some(status & IRQ_STA2_LINE_MASK)
    }
}
