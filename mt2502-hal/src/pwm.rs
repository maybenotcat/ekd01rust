//! PWM register access without board pin routing or brightness policy.
use crate::{mmio::*, registers::*};

/// PWM4 peripheral. GPIO routing is configured independently through gpio::Pin.
pub struct Pwm4 {
    _private: (),
}
pub const CONTROL_DIVIDE_BY_8: u16 = 3;
pub struct Configuration {
    pub control: u16,
    pub period_count: u16,
    pub duty_count: u16,
}
impl Pwm4 {
    /// # Safety
    /// The caller must be the sole owner of PWM4 for this object's lifetime.
    /// Claiming the channel performs no register access.
    pub const unsafe fn claim() -> Self {
        Self { _private: () }
    }
    /// Write the hardware control word, terminal count and duty count in order.
    /// The terminal count is inclusive: count 80 describes 81 timer steps.
    /// Clock gating is separate so the caller can retain its startup ordering.
    pub fn configure(&mut self, configuration: Configuration) {
        write16(PWM4_CTRL, configuration.control);
        write16(PWM4_COUNT, configuration.period_count);
        write16(PWM4_DUTY, configuration.duty_count);
    }
    pub fn enable_clock(&mut self) {
        write32(DRVPDN_CON2_CLR, EKD_PDN_PWM4);
    }
    pub fn set_duty(&mut self, duty_count: u16) {
        write16(PWM4_DUTY, duty_count);
    }
}

/// The verified legacy PWM3 control register. No unverified counter layout is
/// inferred from PWM4; expose only the register currently used by the board.
pub struct Pwm3 {
    _private: (),
}
impl Pwm3 {
    /// # Safety
    /// The caller must exclusively own PWM3. Claiming does not configure it.
    pub const unsafe fn claim() -> Self {
        Self { _private: () }
    }
    pub fn set_control(&mut self, control: u16) {
        write16(EKD_PWM3_CTRL, control);
    }
}
