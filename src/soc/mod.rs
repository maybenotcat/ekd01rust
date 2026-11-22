pub mod backlight;
pub mod bridge_uart;
pub mod btif;
pub mod buttons;
pub mod critical;
pub mod display;
pub mod flash;
pub mod gpio;
pub mod memory;
pub mod mmio;
mod nor_geometry;
mod panels;
pub mod pmu;
mod pmu_data;
pub mod registers;
mod ring;
pub mod rtc;
pub mod time;

use core::fmt;
use mmio::{read16, write16};
use registers::*;

pub struct Watchdog;
impl Watchdog {
    pub fn feed() {
        write16(WDT_RESTART, WDT_RESTART_KEY as u16);
    }
}

pub struct DebugUart;
impl DebugUart {
    fn put(byte: u8) -> bool {
        if read16(UART_LCR(UART1_base)) & UART_LCR_DLAB as u16 != 0 {
            return false;
        }
        for _ in 0..100_000 {
            if read16(UART_LSR(UART1_base)) & UART_LSR_THRE as u16 != 0 {
                write16(UART_THR(UART1_base), byte as u16);
                return true;
            }
        }
        false
    }
    pub fn log(text: &str) {
        critical_section::with(|_| {
            for byte in text.bytes() {
                if byte == b'\n' && !Self::put(b'\r') {
                    break;
                }
                if !Self::put(byte) {
                    break;
                }
            }
        });
    }
    pub fn hex(name: &str, value: u32) {
        use fmt::Write;
        critical_section::with(|_| {
            let _ = writeln!(Self, "{name}={value:08X}");
        });
    }
}
impl fmt::Write for DebugUart {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        Self::log(text);
        Ok(())
    }
}
