use super::{DebugUart, gpio::Pin, mmio::*, registers::*};
use crate::board::pins;

pub struct Backlight {
    _gates: [Pin; 2],
    _pwm: Pin,
    level: u8,
}
impl Backlight {
    pub fn initialize() -> Self {
        let mut gates = unsafe { pins::BACKLIGHT_GATES.map(|number| Pin::claim(number)) };
        for gate in &mut gates {
            gate.set(true);
            gate.output();
            gate.mode(0);
            gate.disable_pull();
        }
        let mut pwm = unsafe { Pin::claim(pins::BACKLIGHT_PWM) };
        pwm.input();
        pwm.disable_pull();
        pwm.mode(2);
        critical_section::with(|_| {
            write16(EKD_ADIE_CLOCK_CLEAR, EKD_ADIE_ISINK_26M as u16);
            update16(ISINKS_CON0, ISINKS_MODE_MASK, 0);
            update16(ISINK0_CON0, EKD_ISINK_CONFIG_MASK, EKD_ISINK_24MA_PWM);
            update16(ISINK1_CON0, EKD_ISINK_CONFIG_MASK, EKD_ISINK_24MA_PWM);
            write16(EKD_PWM3_CTRL, EKD_PWM3_CLOCK_QUERY_DEFAULT as u16);
            write16(PWM4_CTRL, EKD_PWM4_DIV8_CONTROL as u16);
            write16(PWM4_COUNT, EKD_PWM4_20KHZ_COUNT as u16);
            write16(PWM4_DUTY, EKD_PWM4_DEFAULT_DUTY as u16);
            write32(DRVPDN_CON2_CLR, EKD_PDN_PWM4);
            update16(ISINK0_CON0, 0, ISINK_EN_MASK);
            update16(ISINK1_CON0, 0, ISINK_EN_MASK);
        });
        DebugUart::log("[LCD] BACKLIGHT_PWM4_GPIO19\n");
        Self {
            _gates: gates,
            _pwm: pwm,
            level: 3,
        }
    }
    pub fn set_level(&mut self, level: u8) {
        if (1..=5).contains(&level) {
            write16(
                PWM4_DUTY,
                (EKD_PWM4_DUTY_STEPS * EKD_PWM4_BRIGHTNESS_STEP_PERCENT * level as u32 / 100)
                    as u16,
            );
            self.level = level;
        }
    }
    pub fn off(&mut self) {
        write16(PWM4_DUTY, 0);
    }
}
