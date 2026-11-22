use crate::protocol::{CanFrame, MOTOR_BASIC_SETTINGS, MOTOR_ELECTRIC, MOTOR_RIDE};

#[derive(Clone, Copy, Default)]
pub struct Telemetry {
    pub valid: u32,
    pub ride_tick: u32,
    pub electric_tick: u32,
    pub limit_tick: u32,
    pub trip_m: u32,
    pub travel_m: u32,
    pub speed: u16,
    pub voltage: u16,
    pub range: u16,
    pub wheel: u16,
    pub circumference: u16,
    pub limit: u16,
    pub current: i16,
    pub controller_c: i16,
    pub motor_c: i16,
    previous_distance: u16,
    have_distance: bool,
    pub battery: u8,
    pub cadence: u8,
    pub error: u32,
    pub error_serial: u32,
    pub errors: [u32; 10],
    pub history_serial: u32,
}
impl Telemetry {
    pub fn decode(&mut self, frame: &CanFrame, now: u32) -> bool {
        let d = &frame.data;
        let source = frame.id >> 24;
        let command = (frame.id >> 8) & 255;
        let sub = frame.id & 255;
        if matches!(source, 1 | 2 | 4 | 5)
            && (frame.id >> 16) & 255 == 255
            && (1..=8).contains(&frame.length)
            && sub <= 3
            && ((0x11..=0x17).contains(&command) || command == 0x1e)
        {
            if matches!(d[0], 1 | 3 | 6) {
                return false;
            }
            let origin = (command << 8) | (sub << 16) | (source << 24);
            if d[0] == 0 && self.error & 0xffffff00 != origin {
                return true;
            }
            let error = if d[0] != 0 { origin | d[0] as u32 } else { 0 };
            if self.error != error {
                self.error = error;
                self.error_serial = self.error_serial.wrapping_add(1);
                if error != 0 && error != self.errors[0] {
                    self.errors.copy_within(0..9, 1);
                    self.errors[0] = error;
                    self.history_serial = self.history_serial.wrapping_add(1);
                }
            }
            return true;
        }
        let le = |offset: usize| u16::from_le_bytes([d[offset], d[offset + 1]]);
        match (frame.id, frame.length) {
            (MOTOR_RIDE, 8) => {
                let distance = le(1);
                if self.have_distance {
                    let delta = distance.wrapping_sub(self.previous_distance) as i16;
                    if delta > 0 {
                        let metres = delta as u32 * 10;
                        if let Some(value) = self.trip_m.checked_add(metres) {
                            self.trip_m = value;
                        }
                        if let Some(value) = self.travel_m.checked_add(metres) {
                            self.travel_m = value;
                        }
                    }
                }
                self.previous_distance = distance;
                self.have_distance = true;
                self.battery = d[0];
                self.cadence = d[3];
                self.range = le(6);
                self.ride_tick = now;
                self.valid |= 1;
            }
            (MOTOR_ELECTRIC, 8) => {
                self.speed = le(0);
                self.current = le(2) as i16;
                self.voltage = le(4);
                self.controller_c = d[6] as i16 - 40;
                self.motor_c = d[7] as i16 - 40;
                self.electric_tick = now;
                self.valid |= 2;
            }
            (MOTOR_BASIC_SETTINGS, 6) => {
                self.limit = le(0);
                let wheel = le(2);
                self.wheel = (wheel >> 4) * 10 + (wheel & 15);
                self.circumference = le(4);
                self.limit_tick = now;
                self.valid |= 4;
            }
            _ => return false,
        }
        true
    }
    pub fn ride_fresh(&self, now: u32) -> bool {
        self.valid & 1 != 0 && now.wrapping_sub(self.ride_tick) < 640
    }
    pub fn electric_fresh(&self, now: u32) -> bool {
        self.valid & 2 != 0 && now.wrapping_sub(self.electric_tick) < 384
    }
    pub fn reset_trip(&mut self) {
        self.trip_m = 0;
    }
}
