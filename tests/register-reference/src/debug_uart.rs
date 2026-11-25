//! UART1 hardware access. Binaries choose formatting and critical-section policy.
use crate::{mmio::*, registers::*};
pub struct Uart1;
impl Uart1 {
    pub fn put(byte: u8) -> bool {
        if read16(UART_LCR(UART1_base)) & UART_LCR_DLAB as u16 != 0 {
            return false;
        }
        Self::put_boot(byte)
    }
    /// Loader transmission preserves its original readiness check without DLAB probing.
    pub fn put_boot(byte: u8) -> bool {
        for _ in 0..100_000 {
            if read16(UART_LSR(UART1_base)) as u32 & UART_LSR_THRE != 0 {
                write16(UART_THR(UART1_base), byte as u16);
                return true;
            }
        }
        false
    }
}
