use crate::registers::*;
use crate::{
    mmio::{write16, write32},
    registers::{WDT_RESTART, WDT_RESTART_KEY},
};
pub struct Watchdog;
impl Watchdog {
    pub fn feed() {
        // The restart register requires a word transaction, including on MT6261.
        write32(WDT_RESTART, WDT_RESTART_KEY);
    }
}

impl Watchdog {
    pub fn arm_application() {
        const WDT_MODE: u32 = RGU_base;
        const WDT_LENGTH: u32 = RGU_base + 4;
        const WDT_LENGTH_MAX_WITH_KEY: u16 = 0xffe8;
        const WDT_MODE_KEY: u16 = 0x2200;
        const WDT_MODE_ENABLE: u16 = 1;
        // Internal watchdog reset, no external reset output and no IRQ-only mode.
        write16(WDT_LENGTH, WDT_LENGTH_MAX_WITH_KEY);
        write16(WDT_MODE, WDT_MODE_KEY | WDT_MODE_ENABLE);
        Watchdog::feed();
    }
}
