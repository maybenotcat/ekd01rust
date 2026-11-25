use super::{gpio::Pin, mmio::*, registers::*};
use crate::environment::Trace;
use core::marker::PhantomData;

#[derive(Clone, Copy)]
pub struct ClockTime {
    pub hour: u8,
    pub minute: u8,
}
pub struct Rtc<P: Trace> {
    latch: u32,
    _environment: PhantomData<P>,
}
impl<P: Trace> Rtc<P> {
    pub const fn new(latch: u32) -> Self {
        Self {
            latch,
            _environment: PhantomData,
        }
    }
    fn ready() -> bool {
        (0..100_000).any(|_| read16(RTC_BBPU) as u32 & EKD_RTC_CBUSY == 0)
    }
    fn commit() -> bool {
        write16(RTC_WRTGR, RTC_WRTGR_WRTGR as u16);
        Self::ready()
    }
    fn unlock() -> bool {
        write16(RTC_PROT, RTC_PROTECT1 as u16);
        if !Self::commit() {
            return false;
        }
        write16(RTC_PROT, RTC_PROTECT2 as u16);
        Self::commit()
    }
    pub fn read(&self) -> Option<ClockTime> {
        for _ in 0..3 {
            let minute = read16(RTC_TC_MIN) as u32 & RTC_MINUTE_MASK;
            let hour = read16(RTC_TC_HOU) as u32 & RTC_HOUR_MASK;
            if minute != read16(RTC_TC_MIN) as u32 & RTC_MINUTE_MASK {
                continue;
            }
            return if hour < 24 && minute < 60 {
                Some(ClockTime {
                    hour: hour as u8,
                    minute: minute as u8,
                })
            } else {
                None
            };
        }
        None
    }
    pub fn set(&mut self, time: ClockTime) -> bool {
        if time.hour >= 24 || time.minute >= 60 {
            return false;
        }
        critical_section::with(|_| {
            if !Self::ready() || !Self::unlock() {
                return false;
            }
            write16(RTC_TC_HOU, time.hour as u16);
            write16(RTC_TC_MIN, time.minute as u16);
            write16(RTC_TC_SEC, 0);
            Self::commit()
        })
    }
    fn oscillator_shutdown() -> bool {
        update16(RTC_BBPU, 0, EKD_RTC_RELOAD_KEY);
        if !Self::commit() {
            return false;
        }
        let osc = read16(RTC_XOSCCAL) as u32;
        if osc & EKD_RTC_OSC_EMBEDDED == 0 {
            return true;
        }
        for value in [
            EKD_RTC_OSC_UNLOCK1,
            EKD_RTC_OSC_UNLOCK2,
            (osc & !EKD_RTC_OSC_MODE_MASK) | EKD_RTC_OSC_SHUTDOWN_MODE,
        ] {
            write16(RTC_XOSCCAL, value as u16);
            if !Self::ready() {
                return false;
            }
        }
        update16(RTC_BBPU, 0, EKD_RTC_RELOAD_KEY);
        Self::commit()
    }
    pub fn release_power(&mut self) -> bool {
        critical_section::with(|_| {
            P::log("[POWER] SHUTDOWN_ENTER\n");
            if !Self::ready() || !Self::unlock() || !Self::oscillator_shutdown() {
                return false;
            }
            let mut latch = unsafe { Pin::claim(self.latch) };
            latch.set(false);
            latch.output();
            latch.mode(0);
            update16(TEST_CON0, EKD_PMU_PWKEY_LONG_PRESS_EN, 0);
            update16(TEST_CON0, EKD_PMU_PWKEY_TRIGGER, 0);
            update16(TEST_CON0, 0, EKD_PMU_PWKEY_TRIGGER);
            update16(TEST_CON0, EKD_PMU_PWKEY_TRIGGER, 0);
            P::log("[POWER] RTC_RELEASE; USB may retain power\n");
            for _ in 0..100_000 {
                if read16(UART_LSR(UART1_base)) as u32 & UART_LSR_TEMT != 0 {
                    break;
                }
            }
            update16(RTC_BBPU, EKD_RTC_BBPU_POWER_LATCH, EKD_RTC_RELEASE_KEY);
            Self::commit() && read16(RTC_BBPU) as u32 & EKD_RTC_BBPU_POWER_LATCH == 0
        })
    }
}
