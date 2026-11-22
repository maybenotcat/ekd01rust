use super::{DebugUart, gpio::Pin, mmio::*, pmu::Pmu, registers::*};
use crate::board::pins;
use crate::input::{ButtonEvents, ButtonFilter};

pub struct Buttons {
    filter: ButtonFilter,
    row: Pin,
    _columns: [Pin; 2],
    row_started: Option<u32>,
    columns_valid: bool,
    column_state: u8,
    mcu_seen: u8,
    mcu_held: u8,
    queue: [ButtonEvents; 16],
    head: usize,
    tail: usize,
    held: u8,
}
impl Buttons {
    pub fn initialize(now: u32) -> Self {
        let mut columns = unsafe { [Pin::claim(pins::KEY_PLUS), Pin::claim(pins::KEY_MINUS)] };
        for (pin, number) in columns.iter_mut().zip([pins::KEY_PLUS, pins::KEY_MINUS]) {
            pin.input();
            pin.mode(0);
            write32(GPIO_R0_CLR_REG0, 1 << number);
            write32(GPIO_R1_SET_REG0, 1 << number);
            write32(GPIO_PUPD0_CLR, 1 << number);
            write32(GPIO_PULLSEL0_SET, 1 << number);
            write32(GPIO_PULL0_SET, 1 << number);
            update32(GPIO_DINV0, 1 << number, 0);
        }
        let mut row = unsafe { Pin::claim(pins::KEY_ROW) };
        row.set(true);
        row.mode(0);
        row.output();
        DebugUart::log("[KEY] INIT; MCU service07/09 plus20/minus10; PMU POWER\n");
        Self {
            filter: ButtonFilter::new(now),
            row,
            _columns: columns,
            row_started: None,
            columns_valid: false,
            column_state: 0,
            mcu_seen: 0,
            mcu_held: 0,
            queue: [ButtonEvents::default(); 16],
            head: 0,
            tail: 0,
            held: 0,
        }
    }
    fn publish(&mut self, mut event: ButtonEvents) {
        self.held = event.held;
        if event.pressed | event.released | event.short | event.long | event.repeat == 0 {
            return;
        }
        let next = (self.head + 1) & 15;
        if next == self.tail {
            self.tail = self.head;
            event = ButtonEvents {
                held: self.held,
                ..ButtonEvents::default()
            };
        }
        self.queue[self.head] = event;
        self.head = next;
    }
    pub fn mcu(&mut self, code: u8, now: u32) {
        let bit = match code & 0xf0 {
            0x20 => 1,
            0x10 => 2,
            _ => return,
        };
        if !(1..=3).contains(&(code & 15)) {
            return;
        }
        self.mcu_seen |= bit;
        if code & 15 == 3 {
            self.mcu_held &= !bit;
        } else {
            self.mcu_held |= bit;
        }
        if let Some(event) = self.filter.mcu_event(code, now) {
            self.publish(event);
        }
        DebugUart::hex("[KEY] MCU_EVENT", code as u32);
    }
    pub fn service(&mut self, pmu: &Pmu, now: u32) {
        if let Some(started) = self.row_started {
            if now.wrapping_sub(started) >= 2 {
                let pins = read32(GPIO_INPUT_LEVEL0);
                self.column_state = if pins & (1 << crate::board::pins::KEY_PLUS) == 0 {
                    1
                } else {
                    0
                } | if pins & (1 << crate::board::pins::KEY_MINUS) == 0 {
                    2
                } else {
                    0
                };
                self.columns_valid = true;
                self.row.set(true);
                self.row_started = None;
            }
        } else {
            self.row.set(false);
            self.row_started = Some(now);
        }
        let physical = self.column_state | if pmu.power_pressed() { 4 } else { 0 };
        let raw = (physical & !self.mcu_seen) | (self.mcu_held & self.mcu_seen);
        let valid = 4 | self.mcu_seen | if self.columns_valid { 3 } else { 0 };
        let event = self.filter.sample(raw, valid, now);
        if event.pressed != 0 {
            DebugUart::hex("[KEY] DOWN", event.pressed as u32);
        }
        if event.released != 0 {
            DebugUart::hex("[KEY] UP", event.released as u32);
        }
        if event.long != 0 {
            DebugUart::hex("[KEY] LONG", event.long as u32);
        }
        self.publish(event);
    }
    pub fn take(&mut self, now: u32) -> ButtonEvents {
        if self.tail == self.head {
            return ButtonEvents {
                held: self.held,
                queued_tick: now,
                ..ButtonEvents::default()
            };
        }
        let event = self.queue[self.tail];
        self.tail = (self.tail + 1) & 15;
        event
    }
}
