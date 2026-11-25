use crate::{
    mmio::write32,
    registers::{WDT_RESTART, WDT_RESTART_KEY},
};
pub struct Watchdog;
impl Watchdog {
    pub fn feed() {
        // The restart register requires a word transaction, including on MT6261.
        write32(WDT_RESTART, WDT_RESTART_KEY);
    }
}
