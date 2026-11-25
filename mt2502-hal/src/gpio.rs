use super::{mmio::*, registers::*};

/// An owned pad. Board construction is responsible for assigning each pin once.
pub struct Pin {
    number: u32,
}
impl Pin {
    /// # Safety
    /// The pin number must exist on this SoC and be assigned exclusively to
    /// this owner. The board must not create another owner of the same pad.
    pub const unsafe fn claim(number: u32) -> Self {
        Self { number }
    }
    pub fn mode(&mut self, mode: u32) {
        let shift = GPIO_MODE_SHIFT(self.number);
        update32(
            GPIO_MODE_REG(self.number),
            GPIO_MODE_FIELD_MASK << shift,
            mode << shift,
        );
    }
    pub fn set(&mut self, high: bool) {
        write32(
            GPIO_BANK(GPIO_OUTPUT_LEVEL0, self.number) + if high { 4 } else { 8 },
            GPIO_PIN_MASK(self.number),
        );
    }
    pub fn output(&mut self) {
        write32(
            GPIO_BANK(GPIO_DIR0_SET, self.number),
            GPIO_PIN_MASK(self.number),
        );
    }
    pub fn input(&mut self) {
        write32(
            GPIO_BANK(GPIO_DIR0_CLR, self.number),
            GPIO_PIN_MASK(self.number),
        );
    }
    pub fn disable_pull(&mut self) {
        write32(
            GPIO_BANK(GPIO_PULL0_CLR, self.number),
            GPIO_PIN_MASK(self.number),
        );
    }
    pub fn high(&self) -> bool {
        read32(GPIO_BANK(GPIO_INPUT_LEVEL0, self.number)) & GPIO_PIN_MASK(self.number) != 0
    }
}
