//! Loader policy and diagnostic adapter; hardware operations are provided by HAL.
#![allow(dead_code)]
mod critical;
pub use mt2502_hal::{flash, mmio, registers};
pub struct Environment;
impl mt2502_hal::environment::Trace for Environment {
    fn log(text: &str) {
        DebugUart::log(text);
    }
    fn hex(name: &str, value: u32) {
        DebugUart::hex(name, value);
    }
}
impl mt2502_hal::environment::BootEnvironment for Environment {
    fn stop(message: &str) -> ! {
        crate::stop(message)
    }
}
pub type Hardware = mt2502_hal::boot::startup::Startup<Environment>;

pub struct DebugUart;
impl DebugUart {
    fn put(byte: u8) {
        let _ = mt2502_hal::debug_uart::Uart1::put_boot(byte);
    }
    pub fn log(text: &str) {
        for byte in text.bytes() {
            if byte == b'\n' {
                Self::put(b'\r');
            }
            Self::put(byte);
        }
    }
    pub fn hex(name: &str, value: u32) {
        Self::log(name);
        Self::put(b'=');
        for shift in (0..8).rev() {
            let nibble = ((value >> (shift * 4)) & 15) as u8;
            Self::put(if nibble < 10 {
                b'0' + nibble
            } else {
                b'A' + nibble - 10
            });
        }
        Self::log("\n");
    }
}
