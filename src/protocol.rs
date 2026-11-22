//! Length-delimited N22 MCU/CAN protocol. Delimiter bytes inside payloads are data.

pub const MOTOR_CONTROL: u32 = 0x03106300;
pub const WRITE_BASIC_SETTINGS: u32 = 0x05103203;
pub const MOTOR_RIDE: u32 = 0x02f83200;
pub const MOTOR_ELECTRIC: u32 = 0x02f83201;
pub const MOTOR_BASIC_SETTINGS: u32 = 0x02f83203;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CanFrame {
    pub id: u32,
    pub length: u8,
    pub data: [u8; 8],
}
impl CanFrame {
    pub fn new(id: u32, data: &[u8]) -> Option<Self> {
        if id > 0x1fffffff || data.len() > 8 {
            return None;
        }
        let mut frame = Self {
            id,
            length: data.len() as u8,
            ..Self::default()
        };
        frame.data[..data.len()].copy_from_slice(data);
        Some(frame)
    }
    pub fn encode(&self, out: &mut [u8]) -> Option<usize> {
        let size = self.length as usize + 7;
        if self.id > 0x1fffffff || self.length > 8 || out.len() < size {
            return None;
        }
        out[0] = 0xf5;
        out[1] = self.length + 4;
        out[2..6].copy_from_slice(&self.id.to_le_bytes());
        out[6..size - 1].copy_from_slice(&self.data[..self.length as usize]);
        out[size - 1] = 0x0d;
        Some(size)
    }
    pub fn controls(levels: u8, assist: u8, lights: bool, active: bool, mode: u8) -> Option<Self> {
        let table: &[u8] = match levels {
            3 => &[0, 12, 2, 3],
            5 => &[0, 11, 13, 21, 23, 3],
            9 => &[0, 1, 11, 12, 13, 2, 21, 22, 23, 3],
            _ => return None,
        };
        let mode = *[0, 1, 2, 5].get(mode as usize)?;
        Some(Self {
            id: MOTOR_CONTROL,
            length: 4,
            data: [
                levels | (mode << 4),
                *table.get(assist as usize)?,
                lights as u8,
                active as u8,
                0,
                0,
                0,
                0,
            ],
        })
    }
    pub fn basic_settings(limit: u16, wheel: u16, circumference: u16) -> Option<Self> {
        if !(1000..=5000).contains(&limit)
            || !(100..=290).contains(&wheel)
            || !(700..=3000).contains(&circumference)
        {
            return None;
        }
        let wheel = ((wheel / 10) << 4) | (wheel % 10);
        let mut data = [0; 6];
        data[..2].copy_from_slice(&limit.to_le_bytes());
        data[2..4].copy_from_slice(&wheel.to_le_bytes());
        data[4..].copy_from_slice(&circumference.to_le_bytes());
        Self::new(WRITE_BASIC_SETTINGS, &data)
    }
}

pub const VERSION_QUERY: [u8; 5] = [0xf5, 0x42, 1, 1, 0x0d];
pub fn wheel_circumference(wheel: u16) -> Option<u16> {
    (100..=290)
        .contains(&wheel)
        .then(|| (wheel as u32 * 7_979_645 / 1_000_000) as u16)
}

#[derive(Clone, Copy, Debug)]
pub enum McuMessage {
    Voltage(u32),
    Key(u8),
    Version([u8; 5]),
}
impl McuMessage {
    pub fn decode(payload: &[u8]) -> Option<Self> {
        match payload {
            [11, low, high] => Some(Self::Voltage(2 * u16::from_le_bytes([*low, *high]) as u32)),
            [7 | 9, code]
                if matches!(code & 0xf0, 0x10 | 0x20) && (1..=3).contains(&(code & 15)) =>
            {
                Some(Self::Key(*code))
            }
            [1, 0x11, a, b, c, d, e] => Some(Self::Version([*a, *b, *c, *d, *e])),
            _ => None,
        }
    }
}

pub enum BridgeMessage<'a> {
    Can(CanFrame),
    Service(&'a [u8]),
}
pub struct BridgeParser {
    bytes: [u8; 66],
    used: usize,
    pub accepted: u32,
    pub malformed: u32,
    pub ignored: u32,
}
impl BridgeParser {
    pub const fn new() -> Self {
        Self {
            bytes: [0; 66],
            used: 0,
            accepted: 0,
            malformed: 0,
            ignored: 0,
        }
    }
    pub fn reset(&mut self) {
        self.used = 0;
    }
    pub fn timeout_partial(&mut self) {
        if self.used != 0 {
            self.used = 0;
            self.malformed = self.malformed.wrapping_add(1);
        }
    }
    fn remove(&mut self, count: usize) {
        self.bytes.copy_within(count..self.used, 0);
        self.used -= count;
    }
    pub fn feed(&mut self, byte: u8, mut receive: impl FnMut(BridgeMessage<'_>)) {
        if self.used == self.bytes.len() {
            self.malformed = self.malformed.wrapping_add(1);
            self.remove(1);
        }
        self.bytes[self.used] = byte;
        self.used += 1;
        while self.used != 0 {
            if self.bytes[0] != 0xf5 {
                self.remove(1);
                continue;
            }
            if self.used < 2 {
                return;
            }
            let length = (self.bytes[1] & 63) as usize;
            let channel = self.bytes[1] >> 6;
            if channel == 0 && !(4..=12).contains(&length) {
                self.malformed = self.malformed.wrapping_add(1);
                self.remove(1);
                continue;
            }
            if self.used < length + 3 {
                return;
            }
            if self.bytes[length + 2] != 0x0d {
                self.malformed = self.malformed.wrapping_add(1);
                self.remove(1);
                continue;
            }
            if channel == 0 {
                let id = u32::from_le_bytes(self.bytes[2..6].try_into().unwrap());
                if let Some(frame) = CanFrame::new(id, &self.bytes[6..length + 2]) {
                    self.accepted = self.accepted.wrapping_add(1);
                    receive(BridgeMessage::Can(frame));
                } else {
                    self.malformed = self.malformed.wrapping_add(1);
                }
            } else if channel == 1 && length != 0 {
                receive(BridgeMessage::Service(&self.bytes[2..length + 2]));
            } else {
                self.ignored = self.ignored.wrapping_add(1);
            }
            self.remove(length + 3);
        }
    }
}
