//! EKD01 N22 backlight wiring and brightness policy.
use crate::DebugUart;
use crate::board::pins;
use crate::{
    gpio::Pin,
    pmu::isink::{CONFIGURATION_24MA_PWM, Channel, CurrentSinks},
    pwm::{CONTROL_DIVIDE_BY_8, Configuration, Pwm3, Pwm4},
};

// Board-specific period, startup state and five brightness levels.
const PERIOD_COUNT: u16 = 80;
const DEFAULT_DUTY: u16 = 29;
const DUTY_STEPS: u32 = 81;
const BRIGHTNESS_STEP_PERCENT: u32 = 12;
const LEGACY_PWM3_CONTROL: u16 = 7;

pub struct Backlight {
    _gates: [Pin; 2],
    _pwm_pin: Pin,
    _legacy_pwm: Pwm3,
    _sinks: CurrentSinks,
    pwm: Pwm4,
    level: u8,
}
impl Backlight {
    pub fn initialize() -> Self {
        // Board assigns these pads and peripherals once, after PMU initialization.
        let mut gates = unsafe { pins::BACKLIGHT_GATES.map(|number| Pin::claim(number)) };
        for gate in &mut gates {
            gate.set(true);
            gate.output();
            gate.mode(0);
            gate.disable_pull();
        }
        let mut pwm_pin = unsafe { Pin::claim(pins::BACKLIGHT_PWM) };
        pwm_pin.input();
        pwm_pin.disable_pull();
        pwm_pin.mode(2);
        let mut pwm = unsafe { Pwm4::claim() };
        let mut legacy_pwm = unsafe { Pwm3::claim() };
        let mut sinks = unsafe { CurrentSinks::claim() };
        critical_section::with(|_| {
            // Preserve the tested N22 sequence across PMU and PWM peripherals.
            sinks.enable_26mhz_clock();
            sinks.set_mode(0);
            sinks.configure(Channel::Zero, CONFIGURATION_24MA_PWM);
            sinks.configure(Channel::One, CONFIGURATION_24MA_PWM);
            legacy_pwm.set_control(LEGACY_PWM3_CONTROL);
            pwm.configure(Configuration {
                control: CONTROL_DIVIDE_BY_8,
                period_count: PERIOD_COUNT,
                duty_count: DEFAULT_DUTY,
            });
            pwm.enable_clock();
            sinks.enable(Channel::Zero);
            sinks.enable(Channel::One);
        });
        DebugUart::log("[LCD] BACKLIGHT_PWM4_GPIO19\n");
        Self {
            _gates: gates,
            _pwm_pin: pwm_pin,
            _legacy_pwm: legacy_pwm,
            _sinks: sinks,
            pwm,
            level: 3,
        }
    }
    pub fn set_level(&mut self, level: u8) {
        if (1..=5).contains(&level) {
            self.pwm
                .set_duty((DUTY_STEPS * BRIGHTNESS_STEP_PERCENT * level as u32 / 100) as u16);
            self.level = level;
        }
    }
    pub fn off(&mut self) {
        self.pwm.set_duty(0);
    }
}