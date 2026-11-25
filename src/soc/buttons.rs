use super::{DebugUart, pmu::Pmu};
use crate::board::pins;
use crate::input::{ButtonEvents, ButtonFilter};
use mt2502_hal::key_matrix::KeyMatrix;

pub struct Buttons {
    filter: ButtonFilter,
    matrix: KeyMatrix,
    mcu_seen: u8,
    mcu_held: u8,
    queue: [ButtonEvents; 16],
    head: usize,
    tail: usize,
    held: u8,
}
impl Buttons {
    pub fn initialize(now: u32) -> Self {
        let matrix = KeyMatrix::initialize(pins::KEY_ROW, pins::KEY_PLUS, pins::KEY_MINUS);
        DebugUart::log("[KEY] INIT; MCU service07/09 plus20/minus10; PMU POWER\n");
        Self {
            filter: ButtonFilter::new(now),
            matrix,
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
        let (column_state, columns_valid) = self.matrix.sample(now);
        let physical = column_state | if pmu.power_pressed() { 4 } else { 0 };
        let raw = (physical & !self.mcu_seen) | (self.mcu_held & self.mcu_seen);
        let valid = 4 | self.mcu_seen | if columns_valid { 3 } else { 0 };
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
