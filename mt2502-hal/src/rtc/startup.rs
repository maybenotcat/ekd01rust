#[derive(Debug, Clone, Copy)]
pub struct PowerOnState {
    pub first_on: bool,
    pub configuration_valid: bool,
}
pub struct RtcStartup;
use crate::{mmio::*, registers::*, watchdog::Watchdog};
const RTC_POWERKEY1: u32 = RTC_base + 0x50;
const RTC_POWERKEY2: u32 = RTC_base + 0x54;
const RTC_GPIO: u32 = RTC_base + 0x78;
const RTC_CALI: u32 = RTC_base + 0x70;
const FQMTR_CON0: u32 = PMU_base + 0xff0;
const FQMTR_CON1: u32 = PMU_base + 0xff4;
const FQMTR_CON2: u32 = PMU_base + 0xff8;
const FQMTR_WINSET: u32 = PMU_base + 0xffc;
const FQMTR_BUSY: u32 = 0x8000;
const F32K_COUNTER: u32 = 0xa018_0104;
const RTC_IRQ_STATUS: u32 = RTC_base + 0x04;
const RTC_IRQ_EN: u32 = RTC_base + 0x08;
const RTC_AL_MASK: u32 = RTC_base + 0x10;
const RTC_INFO1: u32 = RTC_base + 0x58;
const RTC_TC_FIELDS: [u32; 7] = [0x14, 0x18, 0x1c, 0x20, 0x28, 0x24, 0x2c];
const RTC_AL_FIELDS: [(u32, u32); 6] = [
    (0x30, 0x3f),
    (0x34, 0x3f),
    (0x38, 0x1f),
    (0x3c, 0x1f),
    (0x44, 0x0f),
    (0x48, 0x7f),
];
const RTC_BBPU_KEY: u32 = 0x4300;
const RTC_GPIO_LPEN: u32 = 0x1016;
const RTC_GPIO_LPRST: u32 = 0x0008;
const RTC_GPIO_VBAT_LPSTA_RAW: u32 = 1;
#[derive(Clone, Copy)]
pub struct Defaults {
    pub time: [u8; 7],
    pub alarm: [u32; 6],
}
impl RtcStartup {
    fn read_time() -> [u8; 7] {
        let mut time = RTC_TC_FIELDS.map(|offset| read16(RTC_base + offset) as u8);
        if read16(RTC_base + RTC_TC_FIELDS[0]) < time[0] as u16 {
            time = RTC_TC_FIELDS.map(|offset| read16(RTC_base + offset) as u8);
        }
        time
    }
    fn valid_time(time: &[u8; 7]) -> bool {
        time[0] < 60
            && time[1] < 60
            && time[2] < 24
            && (1..=31).contains(&time[3])
            && (1..=12).contains(&time[4])
            && time[6] <= 127
    }
    fn write_time(time: &[u8; 7]) -> Result<(), crate::error::Error> {
        for index in [0, 1, 2, 3, 5, 4, 6] {
            write16(RTC_base + RTC_TC_FIELDS[index], time[index] as u16);
        }
        Self::commit()?;
        Ok(())
    }
    fn configuration_valid(defaults: [u32; 6]) -> Result<bool, crate::error::Error> {
        let current = Self::read_time();
        let _ = read16(RTC_IRQ_EN);
        let mut alarm = [0u8; 7];
        for ((offset, _), index) in RTC_AL_FIELDS.into_iter().zip([0, 1, 2, 3, 4, 6]) {
            alarm[index] = read16(RTC_base + offset) as u8;
        }
        if Self::valid_time(&current) && Self::valid_time(&alarm) {
            return Ok(true);
        }

        for _ in 0..10_000_000 {
            for ((offset, mask), value) in RTC_AL_FIELDS.into_iter().zip(defaults) {
                update16(RTC_base + offset, mask, value);
            }
            Self::commit()?;
            if RTC_AL_FIELDS
                .into_iter()
                .zip(defaults)
                .all(|((offset, mask), value)| read16(RTC_base + offset) as u32 & mask == value)
            {
                return Ok(false);
            }
            Watchdog::feed();
        }
        return Err(crate::error::Error::RtcAlarmInitFailed);
    }
    fn set_power_keys() -> Result<(), crate::error::Error> {
        write16(RTC_POWERKEY1, 0xa357);
        write16(RTC_POWERKEY2, 0x67d2);
        Self::commit()?;
        Ok(())
    }
    fn clear_alarm_power_on() -> Result<(), crate::error::Error> {
        write16(
            RTC_BBPU,
            ((read16(RTC_BBPU) as u32 & 0xfe) | RTC_BBPU_KEY) as u16,
        );
        Self::commit()?;
        Ok(())
    }
    #[doc = " Original 1002cf1c, dispatched by RTC command 0815 (not 0820)."]
    #[doc = " It establishes retained RTC state, including low-power detector control."]
    pub fn initialize_first_power_on(
        default_time: [u8; 7],
        correction: impl FnOnce(u32, u32) -> i32,
    ) -> Result<(), crate::error::Error> {
        write16(RTC_CALI, 0x4000);
        Self::commit()?;
        write16(RTC_CALI, 0);
        Self::commit()?;
        Self::write_time(&default_time)?;
        let _ = read16(RTC_IRQ_STATUS);
        write16(RTC_base + 0x24, 1);
        write16(RTC_IRQ_EN, 0);
        update16(RTC_INFO1, 0x80, 0);
        write16(RTC_AL_MASK, 0);
        Self::commit()?;
        Self::clear_alarm_power_on()?;
        Self::set_power_keys()?;
        Self::commit()?;
        let gpio = read16(RTC_GPIO) as u32 & (RTC_GPIO_LPEN | RTC_GPIO_LPRST);
        for _ in 0..0xffff {
            write16(RTC_GPIO, gpio as u16);
            write16(RTC_WRTGR, 1);
            if read16(RTC_GPIO) as u32 & RTC_GPIO_VBAT_LPSTA_RAW == 0 {
                break;
            }
            Watchdog::feed();
        }
        let enabled = (read16(RTC_GPIO) as u32 | RTC_GPIO_LPEN) & !RTC_GPIO_LPRST;
        for value in [enabled, enabled | RTC_GPIO_LPRST, enabled] {
            write16(RTC_GPIO, value as u16);
            Self::commit()?;
            Self::ready()?;
        }
        let _ = read16(RTC_IRQ_STATUS);
        if read16(RTC_XOSCCAL) as u32 & EKD_RTC_OSC_EMBEDDED != 0 {
            let selected_trim = Self::trim()?;
            let measured_frequency = Self::measure_frequency()?;
            // The caller supplies the board-selected correction policy.
            Self::set_correction(correction(selected_trim, measured_frequency))?;
        }
        for _ in 0..10_000_000 {
            let time = Self::read_time();
            if [2, 3, 5, 4]
                .into_iter()
                .all(|index| time[index] == default_time[index])
                && read16(RTC_IRQ_EN) & 0x7fff == 0
                && Self::keys_valid()
                && read16(RTC_AL_MASK) & 0x7fff == 0
                && read16(RTC_BBPU) & 1 == 0
            {
                return Ok(());
            }
            Self::write_time(&default_time)?;
            write16(RTC_AL_MASK, 0);
            write16(RTC_IRQ_EN, 0);
            Self::commit()?;
            write16(
                RTC_BBPU,
                ((read16(RTC_BBPU) as u32 & 0xfe) | RTC_BBPU_KEY) as u16,
            );
            for _ in 0..500 {
                unsafe {
                    core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
                }
            }
            Self::set_power_keys()?;
            Self::commit()?;
            Watchdog::feed();
        }
        return Err(crate::error::Error::RtcFirstPowerOnFailed);
    }
    #[doc = " PW_CMD_POWERON 10028040: query first-on and validate both calendars"]
    #[doc = " before selecting the full first-power-on routine and then latching BBPU."]
    pub fn power_on_state(default_alarm: [u32; 6]) -> Result<PowerOnState, crate::error::Error> {
        let first_on = !Self::keys_valid() || read16(RTC_GPIO) & 0x8000 != 0;
        let configuration_valid = Self::configuration_valid(default_alarm)?;
        Ok(PowerOnState {
            first_on,
            configuration_valid,
        })
    }
    pub fn gpio_control() -> u16 {
        read16(RTC_GPIO)
    }
    pub fn ready() -> Result<(), crate::error::Error> {
        for _ in 0..100_000 {
            if read16(RTC_BBPU) as u32 & EKD_RTC_CBUSY == 0 {
                return Ok(());
            }
            Watchdog::feed();
        }
        return Err(crate::error::Error::RtcBusy);
    }
    pub fn commit() -> Result<(), crate::error::Error> {
        write16(RTC_WRTGR, 1);
        Self::ready()?;
        Ok(())
    }
    pub fn reload() -> Result<(), crate::error::Error> {
        update16(RTC_BBPU, 0, EKD_RTC_RELOAD_KEY);
        Self::commit()?;
        Ok(())
    }
    pub fn unlock() -> Result<(), crate::error::Error> {
        for key in [RTC_PROTECT1, RTC_PROTECT2] {
            write16(RTC_PROT, key as u16);
            Self::commit()?;
        }
        Ok(())
    }
    fn write_oscillator_raw(value: u32) -> Result<(), crate::error::Error> {
        for word in [EKD_RTC_OSC_UNLOCK1, EKD_RTC_OSC_UNLOCK2, value] {
            write16(RTC_XOSCCAL, word as u16);
            Self::ready()?;
        }
        Ok(())
    }
    fn write_oscillator(value: u32) -> Result<(), crate::error::Error> {
        Self::write_oscillator_raw(value)?;
        Self::reload()?;
        Ok(())
    }
    fn keys_valid() -> bool {
        read16(RTC_POWERKEY1) == 0xa357 && read16(RTC_POWERKEY2) == 0x67d2
    }
    #[doc = " BL 70006ac4 and EXT_BL 10031b64 use the same oscillator default path."]
    pub fn initialize_oscillator() -> Result<(), crate::error::Error> {
        Self::initialize_default(false)?;
        Ok(())
    }
    fn initialize_default(reload_after_write: bool) -> Result<(), crate::error::Error> {
        Self::ready()?;
        Ok(if !Self::keys_valid() || read16(RTC_GPIO) & 0x8000 != 0 {
            let embedded = read16(RTC_XOSCCAL) as u32 & EKD_RTC_OSC_EMBEDDED != 0;
            let revision = read32(0x8000_0000) & 0xff;
            let default = if !embedded {
                0x407
            } else if revision < 2 {
                0x42f
            } else {
                0x46f
            };
            Self::write_oscillator_raw(default)?;
            if reload_after_write {
                Self::reload()?;
            }
        })
    }
    fn wait_ticks(ticks: u32) -> Result<(), crate::error::Error> {
        let start = read32(F32K_COUNTER);
        let fast = read32(GPT4_COUNT);
        for _ in 0..5_000_000 {
            if read32(F32K_COUNTER).wrapping_sub(start) > ticks {
                return Ok(());
            }
            if read32(GPT4_COUNT).wrapping_sub(fast) > 100 * EKD_GPT4_TICKS_PER_MS {
                break;
            }
        }
        return Err(crate::error::Error::RtcNo32k);
    }
    fn meter_ready() -> Result<(), crate::error::Error> {
        for _ in 0..100_000 {
            if read16(FQMTR_CON0) as u32 & FQMTR_BUSY == 0 {
                return Ok(());
            }
        }
        return Err(crate::error::Error::FqmtrBusy);
    }
    fn measure(window: u16) -> Result<u32, crate::error::Error> {
        write16(FQMTR_CON0, 0x2000);
        Self::wait_ticks(50)?;
        Self::meter_ready()?;
        write16(FQMTR_WINSET, window);
        write16(FQMTR_CON0, 0x4000);
        Self::wait_ticks(50)?;
        Self::meter_ready()?;
        Self::wait_ticks(10)?;
        Ok(read16(FQMTR_CON2) as u32)
    }
    fn set_trim(trim: u32) -> Result<(), crate::error::Error> {
        Self::write_oscillator((read16(RTC_XOSCCAL) as u32 & !31) | trim)?;
        Ok(())
    }
    #[doc = " EXT_BL 10031a50: binary search, then the original long-window tie-break."]
    fn trim() -> Result<u32, crate::error::Error> {
        write16(FQMTR_CON0, 0x4000);
        write16(FQMTR_CON1, 1);
        let (mut low, mut high) = (0, 31);
        Self::set_trim(high)?;
        let mut upper = Self::measure(1)?;
        Self::set_trim(low)?;
        let mut lower = Self::measure(1)?;
        const TARGET: u32 = 0x633;
        let selected = if lower > TARGET {
            low
        } else if upper < TARGET {
            high
        } else {
            loop {
                let middle = (low + high) / 2;
                if middle == low {
                    break;
                }
                Self::set_trim(middle)?;
                let value = Self::measure(1)?;
                if value == TARGET {
                    low = middle;
                    high = middle;
                    break;
                }
                if value < TARGET {
                    low = middle;
                    lower = value;
                } else {
                    high = middle;
                    upper = value;
                }
            }
            let below = TARGET - lower;
            let above = upper - TARGET;
            if low == high {
                low
            } else if below < above {
                low
            } else if below > above {
                high
            } else {
                Self::set_trim(low)?;
                let low_measurement = Self::measure(16)?;
                Self::set_trim(high)?;
                let high_measurement = Self::measure(16)?;
                if 0x34b1u32.wrapping_sub(low_measurement) > high_measurement.wrapping_sub(0x34b1) {
                    high
                } else {
                    low
                }
            }
        };
        Self::set_trim(selected)?;
        Ok(selected)
    }
    fn measure_frequency() -> Result<u32, crate::error::Error> {
        write16(FQMTR_CON0, 0x4000);
        write16(FQMTR_CON1, 1);
        let count = Self::measure(63)?;
        if count == 0 {
            return Err(crate::error::Error::FqmtrZero);
        }
        Ok((26_000_000u32 * 64) / count)
    }
    fn set_correction(correction: i32) -> Result<(), crate::error::Error> {
        let correction = correction.clamp(-8192, 8191);
        write16(RTC_CALI, ((correction as u32 & 0x3fff) | 0x4000) as u16);
        Self::commit()?;
        Ok(())
    }
    #[doc = " EXT_BL RTC_setXOSC (1002f57c), RTC_HW_Init and RTC power-latch sequence."]
    pub fn restore_running_clock() -> Result<(), crate::error::Error> {
        Self::unlock()?;
        Self::initialize_default(true)?;
        Self::reload()?;
        if read16(RTC_XOSCCAL) as u32 & EKD_RTC_OSC_EMBEDDED != 0 {
            if Self::keys_valid() {
                Self::trim()?;
                let frequency = Self::measure_frequency()?;
                Self::set_correction(32768i32 - frequency as i32)?;
            }
            if read32(0x8000_0000) & 0xff >= 2 {
                Self::write_oscillator(
                    (read16(RTC_XOSCCAL) as u32 & !EKD_RTC_OSC_MODE_MASK) | 0x40,
                )?;
            }
        }
        Self::ready()?;
        Self::reload()?;
        Ok(())
    }
}

impl RtcStartup {
    pub fn enable_power_latch() -> Result<(), crate::error::Error> {
        update16(RTC_BBPU, 0, 0x430c);
        Self::commit()?;
        if read16(RTC_BBPU) as u32 & EKD_RTC_BBPU_POWER_LATCH == 0 {
            return Err(crate::error::Error::PowerLatchFailed);
        }
        Ok(())
    }
    pub fn finish_power_on() -> Result<(), crate::error::Error> {
        update16(RTC_INFO1, 0x0e, 0);
        write16(
            RTC_base + 0x40,
            (read16(RTC_base + 0x40) & 0xff00) | read16(RTC_base + 0x24),
        );
        Self::commit()
    }
}
