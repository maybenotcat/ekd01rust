//! Verified EKD01 N22 wiring. SoC register names live separately in soc::registers.
pub mod pins {
    pub const LCD_RESET: u32 = 38;
    pub const LCD_CS: u32 = 39;
    pub const LCD_CLOCK: u32 = 47;
    pub const LCD_OTHER_BUS: [u32; 3] = [46, 48, 49];
    pub const BACKLIGHT_GATES: [u32; 2] = [3, 15];
    pub const BACKLIGHT_PWM: u32 = 19;
    pub const POWER_LATCH: u32 = 45;
    pub const KEY_ROW: u32 = 21;
    pub const KEY_PLUS: u32 = 16;
    pub const KEY_MINUS: u32 = 14;
    pub const MCU_RX: u32 = 0;
    pub const MCU_TX: u32 = 1;
}