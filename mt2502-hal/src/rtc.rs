pub mod startup;
use crate::{error::Error, mmio::*, registers::*};

#[derive(Clone, Copy)]
pub struct ClockTime {
    pub hour: u8,
    pub minute: u8,
}
pub struct Rtc;
impl Rtc {
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
    pub fn try_set(&mut self, time: ClockTime) -> Result<bool, Error> {
        if time.hour >= 24 || time.minute >= 60 {
            return Ok(false);
        }
        critical_section::with(|_| {
            if !Self::ready() || !Self::unlock() {
                return Err(Error::RtcBusy);
            }
            write16(RTC_TC_HOU, time.hour as u16);
            write16(RTC_TC_MIN, time.minute as u16);
            write16(RTC_TC_SEC, 0);
            if !Self::commit() {
                return Err(Error::RtcBusy);
            }
            Ok(true)
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
    pub fn prepare_shutdown(&self) -> Result<(), Error> {
        if !Self::ready() || !Self::unlock() || !Self::oscillator_shutdown() {
            return Err(Error::RtcBusy);
        }
        Ok(())
    }
    pub fn release_power_latch(&self) -> Result<(), Error> {
        update16(RTC_BBPU, EKD_RTC_BBPU_POWER_LATCH, EKD_RTC_RELEASE_KEY);
        if !Self::commit() {
            return Err(Error::RtcBusy);
        }
        if read16(RTC_BBPU) as u32 & EKD_RTC_BBPU_POWER_LATCH != 0 {
            return Err(Error::PowerLatchFailed);
        }
        Ok(())
    }
}
