//! PMU current-sink bank. Channel selection and operating policy belong to callers.
use crate::{mmio::*, registers::*};

#[derive(Clone, Copy)]
pub enum Channel {
    Zero,
    One,
}
impl Channel {
    fn control(self) -> u32 {
        match self {
            Self::Zero => ISINK0_CON0,
            Self::One => ISINK1_CON0,
        }
    }
}
/// Owns the shared mode/clock configuration and both current-sink channels.
pub struct CurrentSinks {
    _private: (),
}
pub const CONFIGURATION_24MA_PWM: u16 = 0x50;
impl CurrentSinks {
    /// # Safety
    /// The caller must exclusively own the current-sink bank. PMU startup must
    /// finish before acquisition. Claiming the bank performs no register access.
    pub const unsafe fn claim() -> Self {
        Self { _private: () }
    }
    pub fn enable_26mhz_clock(&mut self) {
        write16(EKD_ADIE_CLOCK_CLEAR, EKD_ADIE_ISINK_26M as u16);
    }
    pub fn set_mode(&mut self, mode_bits: u16) {
        update16(ISINKS_CON0, ISINKS_MODE_MASK, mode_bits as u32);
    }
    /// Program the channel's current/mode/enable fields. No implicit enable.
    pub fn configure(&mut self, channel: Channel, configuration: u16) {
        update16(
            channel.control(),
            EKD_ISINK_CONFIG_MASK,
            configuration as u32,
        );
    }
    pub fn enable(&mut self, channel: Channel) {
        update16(channel.control(), 0, ISINK_EN_MASK);
    }
}
