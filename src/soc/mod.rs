//! Binary integration and EKD01 policy; peripheral implementations live in the HAL.
pub mod buttons;
pub mod critical;
pub mod flash;
pub mod memory;
pub mod time;
pub use mt2502_hal::{mmio, registers, watchdog::Watchdog};

pub struct Environment;
impl mt2502_hal::environment::Trace for Environment {
    fn log(text: &str) {
        DebugUart::log(text);
    }
    fn hex(name: &str, value: u32) {
        DebugUart::hex(name, value);
    }
}
impl mt2502_hal::environment::Runtime for Environment {
    fn now() -> u32 {
        time::Timer::now()
    }
    async fn delay_ticks(ticks: u64) {
        embassy_time::Timer::after_ticks(ticks).await;
    }
}
pub mod backlight;
pub mod bridge_uart {
    pub type BridgeUart = mt2502_hal::uart3::Uart3<super::Environment>;
}
pub mod btif {
    pub type Btif = mt2502_hal::btif::Btif<super::Environment>;
    pub use mt2502_hal::btif::BtifError;
}
pub mod pmu {
    pub type Pmu = mt2502_hal::pmu::Pmu<super::Environment>;
}
pub mod rtc {
    pub type Rtc = mt2502_hal::rtc::Rtc<super::Environment>;
    pub use mt2502_hal::rtc::ClockTime;
}
pub mod display {
    pub type Display = mt2502_hal::display::Display<super::Environment>;
    pub use mt2502_hal::display::{DisplayError, DisplayPins, HEIGHT, WIDTH};
}

use core::fmt;

pub struct DebugUart;
impl DebugUart {
    fn put(byte: u8) -> bool {
        mt2502_hal::debug_uart::Uart1::put(byte)
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
