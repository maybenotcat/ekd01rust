//! PMU mechanisms. Board code supplies settings and chooses their ordering.
use crate::{mmio::*, registers::*};

pub fn boot_flags() -> u32 {
    read16(EKD_BOOT_FLAGS) as u32
}
pub fn apply_settings(settings: &[(u32, u32, u32)]) {
    for &(offset, mask, bits) in settings {
        update16(PMU_base + offset, mask, bits);
    }
}
pub struct Charger;
impl Charger {
    pub fn set_high_voltage_threshold(&mut self, index: u32) {
        update16(CHR_CON0, RG_VCDT_HV_VTH_MASK, index << RG_VCDT_HV_VTH_SHIFT);
    }
    pub fn enable_high_voltage_detection(&mut self) {
        update16(CHR_CON0, RG_VCDT_HV_EN_MASK, RG_VCDT_HV_EN_MASK);
    }
    pub fn input_status(&self) -> bool {
        read16(CHR_CON0 + 4) & 8 != 0
    }
    pub fn battery_status_clear(&self) -> bool {
        read16(CHR_CON0) & 0x20 == 0
    }
    pub fn set_charge_current_index(&mut self, index: u32) {
        update16(CHR_CON0, 0xf000, index << 12);
        update16(CHR_CON0 + 8, 15, index);
    }
    pub fn enable(&mut self) {
        update16(CHR_CON0, 1, 1);
        update16(CHR_CON0, 8, 8);
    }
    pub fn release_usb_download_power(&mut self) {
        update16(CHR_CON10, 0x4000, 0x4000);
    }
}
pub fn enable_adie_interrupts() {
    update16(0xa0730010, 0, 0x10);
}
pub fn set_keypad_led_mode(mode: u32) {
    update16(KPLED_CON0, KPLED_MODE_MASK, mode);
}
pub fn set_speaker_volume_step(step: u32) {
    update16(SPK_CON0, RG_SPK_VOL_MASK, step << RG_SPK_VOL_SHIFT);
}
pub struct PowerKey;
impl PowerKey {
    pub fn configure_reset(&mut self) {
        update16(TEST_CON0, RG_PWRKEY_RST_EN_MASK, RG_PWRKEY_RST_EN_MASK);
        update16(
            TEST_CON0,
            RG_LNGP_SHUTDOWN_SEL_MASK,
            RG_LNGP_SHUTDOWN_SEL_MASK,
        );
        for bits in [0, RG_PWRKEY_RST_FUNC_SET_MASK, 0] {
            update16(TEST_CON0, RG_PWRKEY_RST_FUNC_SET_MASK, bits);
        }
    }
    pub fn disable_long_press_and_pulse(&mut self) {
        update16(TEST_CON0, EKD_PMU_PWKEY_LONG_PRESS_EN, 0);
        for value in [0, EKD_PMU_PWKEY_TRIGGER, 0] {
            update16(TEST_CON0, EKD_PMU_PWKEY_TRIGGER, value);
        }
    }
}
