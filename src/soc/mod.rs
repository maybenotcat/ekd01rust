pub mod backlight;
pub mod bridge_uart;
pub mod btif;
pub mod buttons;
pub mod critical;
pub mod display;
pub use mt2502_hal::flash;
pub mod gpio;
pub mod memory;
pub use mt2502_hal::mmio;
mod panels;
pub mod pmu;
use mt2502_hal::pmu::data as pmu_data;
pub use mt2502_hal::registers;
mod ring;
pub mod rtc;
pub mod time;

use core::fmt;
use mmio::{read16, write16, write32};
use registers::*;

pub struct Watchdog;
impl Watchdog {
    pub fn feed() {
        // MT6261 DRV_WDT_RETN_REG requires a 32-bit restart transaction.
        write32(WDT_RESTART, WDT_RESTART_KEY);
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

// Static diagnostic bridge used only during shared-crate extraction.
#[unsafe(no_mangle)]
fn mt2502_diagnostic_hex(name: &str, value: u32) { DebugUart::hex(name, value); }