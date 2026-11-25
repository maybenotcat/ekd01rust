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

impl Uart1 {
    pub fn initialize(divisor: u16) {
        // Configure 8N1 with the divisor chosen for the caller's UART clock.
        write16(UART1_base + 0x0c, 0x83);
        write16(UART1_base, divisor);
        write16(UART1_base + 4, 0);
        write16(UART1_base + 0x0c, 3);
        write16(UART1_base + 8, 0x47);
        write16(UART1_base + 0x10, 3);
        write16(UART1_base + 4, 0);
        write16(UART1_base + 0x24, 0);
    }
}
impl Uart1 {
    pub fn wait_transmitter_empty(limit: u32) -> bool {
        for _ in 0..limit {
            if read16(UART_LSR(UART1_base)) as u32 & UART_LSR_TEMT != 0 {
                return true;
            }
        }
        false
    }
    pub fn wait_transmitter_empty_feeding_watchdog() {
        while read16(UART_LSR(UART1_base)) as u32 & UART_LSR_TEMT == 0 {
            crate::watchdog::Watchdog::feed();
        }
    }
}
