//! EKD01 clock presentation and ordered board power-off.
use core::marker::PhantomData;
pub use mt2502_hal::rtc::ClockTime;
use mt2502_hal::{debug_uart::Uart1, environment::Trace, gpio::Pin, pmu::control::PowerKey};
pub struct Rtc<P: Trace> {
    clock: mt2502_hal::rtc::Rtc,
    _environment: PhantomData<P>,
}
impl<P: Trace> Rtc<P> {
    pub const fn new() -> Self {
        Self {
            clock: mt2502_hal::rtc::Rtc,
            _environment: PhantomData,
        }
    }
    pub fn read(&self) -> Option<ClockTime> {
        self.clock.read()
    }
    pub fn set(&mut self, time: ClockTime) -> bool {
        self.clock.try_set(time).unwrap_or(false)
    }
    pub fn release_power(&mut self) -> bool {
        critical_section::with(|_| {
            P::log("[POWER] SHUTDOWN_ENTER\n");
            if self.clock.prepare_shutdown().is_err() {
                return false;
            }
            // The board assigns the physical latch output; the RTC driver does not.
            let mut latch = unsafe { Pin::claim(crate::pins::POWER_LATCH) };
            latch.set(false);
            latch.output();
            latch.mode(0);
            PowerKey.disable_long_press_and_pulse();
            P::log("[POWER] RTC_RELEASE; USB may retain power\n");
            let _ = Uart1::wait_transmitter_empty(100_000);
            self.clock.release_power_latch().is_ok()
        })
    }
}
