use crate::{gpio::Pin, mmio::*, registers::*};
pub struct KeyMatrix {
    row: Pin,
    _columns: [Pin; 2],
    plus: u32,
    minus: u32,
    row_started: Option<u32>,
    columns_valid: bool,
    column_state: u8,
}
impl KeyMatrix {
    pub fn initialize(row_pin: u32, plus: u32, minus: u32) -> Self {
        let mut columns = unsafe { [Pin::claim(plus), Pin::claim(minus)] };
        for (pin, number) in columns.iter_mut().zip([plus, minus]) {
            pin.input();
            pin.mode(0);
            write32(GPIO_R0_CLR_REG0, 1 << number);
            write32(GPIO_R1_SET_REG0, 1 << number);
            write32(GPIO_PUPD0_CLR, 1 << number);
            write32(GPIO_PULLSEL0_SET, 1 << number);
            write32(GPIO_PULL0_SET, 1 << number);
            update32(GPIO_DINV0, 1 << number, 0);
        }
        let mut row = unsafe { Pin::claim(row_pin) };
        row.set(true);
        row.mode(0);
        row.output();
        Self {
            row,
            _columns: columns,
            plus,
            minus,
            row_started: None,
            columns_valid: false,
            column_state: 0,
        }
    }
    /// Preserve the two-tick row settling interval before sampling columns.
    pub fn sample(&mut self, now: u32) -> (u8, bool) {
        if let Some(started) = self.row_started {
            if now.wrapping_sub(started) >= 2 {
                let pins = read32(GPIO_INPUT_LEVEL0);
                self.column_state = if pins & (1 << self.plus) == 0 { 1 } else { 0 }
                    | if pins & (1 << self.minus) == 0 { 2 } else { 0 };
                self.columns_valid = true;
                self.row.set(true);
                self.row_started = None;
            }
        } else {
            self.row.set(false);
            self.row_started = Some(now);
        }
        (self.column_state, self.columns_valid)
    }
}
