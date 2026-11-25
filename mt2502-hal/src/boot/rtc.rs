use crate::environment::BootEnvironment;
pub struct RtcBoot<P: BootEnvironment>(core::marker::PhantomData<P>);
// RTC/F32K hardware path from the original N22 BL and EXT_BL.
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
const DEFAULT_TIME: [u8; 7] = [0, 0, 0, 1, 1, 1, 21];

impl<P: BootEnvironment> RtcBoot<P> {
    fn read_time() -> [u8; 7] {
        let mut time = RTC_TC_FIELDS.map(|offset| read16(RTC_base + offset) as u8);
        // Original 100345dc rereads all fields when seconds roll over.
        if read16(RTC_base + RTC_TC_FIELDS[0]) < time[0] as u16 {
            time = RTC_TC_FIELDS.map(|offset| read16(RTC_base + offset) as u8);
        }
        time
    }

    fn valid_time(time: &[u8; 7]) -> bool {
        // Original 10034aa0 validates ranges, not month-specific day counts.
        time[0] < 60
            && time[1] < 60
            && time[2] < 24
            && (1..=31).contains(&time[3])
            && (1..=12).contains(&time[4])
            && time[6] <= 127
    }

    fn write_time(time: &[u8; 7]) {
        for index in [0, 1, 2, 3, 5, 4, 6] {
            write16(RTC_base + RTC_TC_FIELDS[index], time[index] as u16);
        }
        Self::commit();
    }

    fn configuration_valid() -> bool {
        let current = Self::read_time();
        let _ = read16(RTC_IRQ_EN);
        let mut alarm = [0u8; 7];
        for ((offset, _), index) in RTC_AL_FIELDS.into_iter().zip([0, 1, 2, 3, 4, 6]) {
            // 1002f358 reads the complete halfword, then truncates to a byte.
            alarm[index] = read16(RTC_base + offset) as u8;
        }
        if Self::valid_time(&current) && Self::valid_time(&alarm) {
            return true;
        }
        // 1002d3e4 resets an invalid alarm while preserving its spare bits.
        let defaults = [0, 0, 0, 1, 1, 0];
        for _ in 0..10_000_000 {
            for ((offset, mask), value) in RTC_AL_FIELDS.into_iter().zip(defaults) {
                update16(RTC_base + offset, mask, value);
            }
            Self::commit();
            if RTC_AL_FIELDS
                .into_iter()
                .zip(defaults)
                .all(|((offset, mask), value)| read16(RTC_base + offset) as u32 & mask == value)
            {
                return false;
            }
            Watchdog::feed();
        }
        P::stop("[BOOT] RTC_ALARM_INIT_FAILED\n");
    }

    fn set_power_keys() {
        write16(RTC_POWERKEY1, 0xa357);
        write16(RTC_POWERKEY2, 0x67d2);
        Self::commit();
    }

    fn clear_alarm_power_on() {
        write16(
            RTC_BBPU,
            ((read16(RTC_BBPU) as u32 & 0xfe) | RTC_BBPU_KEY) as u16,
        );
        Self::commit();
    }

    /// Original 1002cf1c, dispatched by RTC command 0815 (not 0820).
    /// It establishes retained RTC state, including low-power detector control.
    fn initialize_first_power_on() {
        write16(RTC_CALI, 0x4000);
        Self::commit();
        write16(RTC_CALI, 0);
        Self::commit();
        Self::write_time(&DEFAULT_TIME);
        let _ = read16(RTC_IRQ_STATUS);
        write16(RTC_base + 0x24, 1);
        write16(RTC_IRQ_EN, 0);
        update16(RTC_INFO1, 0x80, 0);
        write16(RTC_AL_MASK, 0);
        Self::commit();
        Self::clear_alarm_power_on();
        Self::set_power_keys();
        Self::commit();

        let gpio = read16(RTC_GPIO) as u32 & (RTC_GPIO_LPEN | RTC_GPIO_LPRST);
        for _ in 0..0xffff {
            write16(RTC_GPIO, gpio as u16);
            write16(RTC_WRTGR, 1);
            if read16(RTC_GPIO) as u32 & RTC_GPIO_VBAT_LPSTA_RAW == 0 {
                break;
            }
            Watchdog::feed();
        }
        // 1002f310: enable both low-power detectors and pulse their reset bit.
        let enabled = (read16(RTC_GPIO) as u32 | RTC_GPIO_LPEN) & !RTC_GPIO_LPRST;
        for value in [enabled, enabled | RTC_GPIO_LPRST, enabled] {
            write16(RTC_GPIO, value as u16);
            Self::commit();
            Self::ready();
        }
        let _ = read16(RTC_IRQ_STATUS);

        if read16(RTC_XOSCCAL) as u32 & EKD_RTC_OSC_EMBEDDED != 0 {
            let selected_trim = Self::trim();
            let _measured_frequency = Self::measure_frequency();
            // N22 1002cfcc loads the selected trim from SP+0x0c, not the
            // measured frequency at SP+0x08. Preserve that binary behavior.
            Self::set_correction(32768 - selected_trim as i32);
        }
        // Original 1002cfe0 readback loop covers the RTC's debounce interval.
        for _ in 0..10_000_000 {
            let time = Self::read_time();
            if [2, 3, 5, 4]
                .into_iter()
                .all(|index| time[index] == DEFAULT_TIME[index])
                && read16(RTC_IRQ_EN) & 0x7fff == 0
                && Self::keys_valid()
                && read16(RTC_AL_MASK) & 0x7fff == 0
                && read16(RTC_BBPU) & 1 == 0
            {
                return;
            }
            Self::write_time(&DEFAULT_TIME);
            write16(RTC_AL_MASK, 0);
            write16(RTC_IRQ_EN, 0);
            Self::commit();
            write16(
                RTC_BBPU,
                ((read16(RTC_BBPU) as u32 & 0xfe) | RTC_BBPU_KEY) as u16,
            );
            for _ in 0..500 {
                unsafe {
                    core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
                }
            }
            Self::set_power_keys();
            Self::commit();
            Watchdog::feed();
        }
        P::stop("[BOOT] RTC_FIRST_POWER_ON_FAILED\n");
    }

    /// PW_CMD_POWERON 10028040: query first-on and validate both calendars
    /// before selecting the full first-power-on routine and then latching BBPU.
    pub fn initialize_power_on_state() {
        let first_on = !Self::keys_valid() || read16(RTC_GPIO) & 0x8000 != 0;
        let valid = Self::configuration_valid();
        P::hex("[BOOT] RTC_FIRST_ON", first_on as u32);
        P::hex("[BOOT] RTC_CONFIG_VALID", valid as u32);
        if first_on || !valid {
            P::log("[BOOT] RTC_FIRST_POWER_ON\n");
            Self::initialize_first_power_on();
        }
        P::hex("[BOOT] RTC_GPIO", read16(RTC_GPIO) as u32);
    }

    pub fn ready() {
        for _ in 0..100_000 {
            if read16(RTC_BBPU) as u32 & EKD_RTC_CBUSY == 0 {
                return;
            }
            Watchdog::feed();
        }
        P::stop("[BOOT] RTC_BUSY\n");
    }

    pub fn commit() {
        write16(RTC_WRTGR, 1);
        Self::ready();
    }

    pub fn reload() {
        update16(RTC_BBPU, 0, EKD_RTC_RELOAD_KEY);
        Self::commit();
    }

    pub fn unlock() {
        for key in [RTC_PROTECT1, RTC_PROTECT2] {
            write16(RTC_PROT, key as u16);
            Self::commit();
        }
    }

    fn write_oscillator_raw(value: u32) {
        for word in [EKD_RTC_OSC_UNLOCK1, EKD_RTC_OSC_UNLOCK2, value] {
            write16(RTC_XOSCCAL, word as u16);
            Self::ready();
        }
    }

    fn write_oscillator(value: u32) {
        Self::write_oscillator_raw(value);
        Self::reload();
    }

    fn keys_valid() -> bool {
        read16(RTC_POWERKEY1) == 0xa357 && read16(RTC_POWERKEY2) == 0x67d2
    }

    /// BL 70006ac4 and EXT_BL 10031b64 use the same oscillator default path.
    pub fn initialize_oscillator() {
        Self::initialize_default(false);
    }

    fn initialize_default(reload_after_write: bool) {
        Self::ready();
        if !Self::keys_valid() || read16(RTC_GPIO) & 0x8000 != 0 {
            let embedded = read16(RTC_XOSCCAL) as u32 & EKD_RTC_OSC_EMBEDDED != 0;
            // Original INT_SW_SecVersion reads the low byte at physical 80000000.
            let revision = read32(0x8000_0000) & 0xff;
            let default = if !embedded {
                0x407
            } else if revision < 2 {
                0x42f
            } else {
                0x46f
            };
            Self::write_oscillator_raw(default);
            if reload_after_write {
                Self::reload();
            }
        }
    }

    fn wait_ticks(ticks: u32) {
        let start = read32(F32K_COUNTER);
        // Independent timeout: a stopped 32k source must not strand the bootloader.
        let fast = read32(GPT4_COUNT);
        for _ in 0..5_000_000 {
            if read32(F32K_COUNTER).wrapping_sub(start) > ticks {
                return;
            }
            if read32(GPT4_COUNT).wrapping_sub(fast) > 100 * EKD_GPT4_TICKS_PER_MS {
                break;
            }
        }
        P::stop("[BOOT] RTC_NO_32K\n");
    }

    fn meter_ready() {
        for _ in 0..100_000 {
            if read16(FQMTR_CON0) as u32 & FQMTR_BUSY == 0 {
                return;
            }
        }
        P::stop("[BOOT] FQMTR_BUSY\n");
    }

    fn measure(window: u16) -> u32 {
        write16(FQMTR_CON0, 0x2000);
        Self::wait_ticks(50);
        Self::meter_ready();
        write16(FQMTR_WINSET, window);
        write16(FQMTR_CON0, 0x4000);
        Self::wait_ticks(50);
        Self::meter_ready();
        Self::wait_ticks(10);
        read16(FQMTR_CON2) as u32
    }

    fn set_trim(trim: u32) {
        Self::write_oscillator((read16(RTC_XOSCCAL) as u32 & !31) | trim);
    }

    /// EXT_BL 10031a50: binary search, then the original long-window tie-break.
    fn trim() -> u32 {
        write16(FQMTR_CON0, 0x4000);
        write16(FQMTR_CON1, 1);
        let (mut low, mut high) = (0, 31);
        Self::set_trim(high);
        let mut upper = Self::measure(1);
        Self::set_trim(low);
        let mut lower = Self::measure(1);
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
                Self::set_trim(middle);
                let value = Self::measure(1);
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
                Self::set_trim(low);
                let low_measurement = Self::measure(16);
                Self::set_trim(high);
                let high_measurement = Self::measure(16);
                if 0x34b1u32.wrapping_sub(low_measurement) > high_measurement.wrapping_sub(0x34b1) {
                    high
                } else {
                    low
                }
            }
        };
        Self::set_trim(selected);
        selected
    }

    fn measure_frequency() -> u32 {
        write16(FQMTR_CON0, 0x4000);
        write16(FQMTR_CON1, 1);
        let count = Self::measure(63);
        if count == 0 {
            P::stop("[BOOT] FQMTR_ZERO\n");
        }
        (26_000_000u32 * 64) / count
    }

    fn set_correction(correction: i32) {
        let correction = correction.clamp(-8192, 8191);
        write16(RTC_CALI, ((correction as u32 & 0x3fff) | 0x4000) as u16);
        Self::commit();
    }

    /// EXT_BL RTC_setXOSC (1002f57c), RTC_HW_Init and RTC power-latch sequence.
    pub fn restore_running_clock() {
        Self::unlock();
        Self::initialize_default(true);
        Self::reload();
        if read16(RTC_XOSCCAL) as u32 & EKD_RTC_OSC_EMBEDDED != 0 {
            if Self::keys_valid() {
                Self::trim();
                // EXT_BL 10031b10 measures the resulting 32k frequency over 64 cycles.
                let frequency = Self::measure_frequency();
                Self::set_correction(32768i32 - frequency as i32);
            }
            // Silicon revisions below 2 reject this command in the original DCL.
            if read32(0x8000_0000) & 0xff >= 2 {
                Self::write_oscillator(
                    (read16(RTC_XOSCCAL) as u32 & !EKD_RTC_OSC_MODE_MASK) | 0x40,
                );
            }
        }
        // Original RTC_HW_Init waits CBUSY and reloads before latching BBPU.
        Self::ready();
        Self::reload();
    }
}
