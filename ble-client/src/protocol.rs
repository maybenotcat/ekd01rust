//! EKD01 telemetry v1: three 20-byte pages with a common wrapping sequence.
use std::time::Duration;
include!(concat!(env!("OUT_DIR"), "/protocol_constants.rs"));

const PAGE_BYTES: usize = 20;
const SNAPSHOT_BYTES: usize = 60;
pub const STALE_AFTER: Duration = Duration::from_secs(2);
fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub sequence: u16,
    pub ticks: u32,
    pub speed: u16,
    pub voltage: u16,
    pub current: i16,
    pub power: i16,
    pub cadence: u8,
    pub battery: u8,
    pub assist: u8,
    pub flags: u8,
    pub trip_metres: u32,
    pub range: u16,
    pub controller_c: i16,
    pub motor_c: i16,
    pub levels: u8,
    pub can_rx: u32,
    pub can_tx: u32,
    pub uart_errors: u32,
    pub malformed: u32,
    pub mcu_rx: u32,
}
impl Snapshot {
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != SNAPSHOT_BYTES {
            return None;
        }
        let sequence = u16_at(bytes, 2);
        if bytes
            .chunks_exact(PAGE_BYTES)
            .enumerate()
            .any(|(page, frame)| {
                frame[0] != 1 || frame[1] != page as u8 + 1 || u16_at(frame, 2) != sequence
            })
        {
            return None;
        }
        Some(Self {
            sequence,
            ticks: u32_at(bytes, 4),
            speed: u16_at(bytes, 8),
            voltage: u16_at(bytes, 10),
            current: u16_at(bytes, 12) as i16,
            power: u16_at(bytes, 14) as i16,
            cadence: bytes[16],
            battery: bytes[17],
            assist: bytes[18],
            flags: bytes[19],
            trip_metres: u32_at(bytes, 24),
            range: u16_at(bytes, 28),
            controller_c: u16_at(bytes, 30) as i16,
            motor_c: u16_at(bytes, 32) as i16,
            levels: bytes[34],
            can_rx: u32_at(bytes, 36),
            can_tx: u32_at(bytes, 44),
            uart_errors: u32_at(bytes, 48),
            malformed: u32_at(bytes, 52),
            mcu_rx: u32_at(bytes, 56),
        })
    }
    pub fn ride_fresh(&self) -> bool {
        self.flags & 1 != 0
    }
    pub fn electric_fresh(&self) -> bool {
        self.flags & 2 != 0
    }
    pub fn adc_fresh(&self) -> bool {
        self.flags & 4 != 0
    }
    pub fn voltage_source(&self) -> &'static str {
        if self.electric_fresh() {
            "CAN"
        } else if self.adc_fresh() {
            "MCU ADC"
        } else {
            "unavailable"
        }
    }
}

pub struct Assembler {
    data: [u8; SNAPSHOT_BYTES],
    sequence: u16,
    mask: u8,
    last: Option<Duration>,
}
impl Default for Assembler {
    fn default() -> Self {
        Self {
            data: [0; SNAPSHOT_BYTES],
            sequence: 0,
            mask: 0,
            last: None,
        }
    }
}
impl Assembler {
    pub fn reset(&mut self) {
        self.mask = 0;
        self.last = None;
    }
    pub fn accept(&mut self, frame: &[u8], now: Duration) -> Option<Snapshot> {
        if frame.len() != PAGE_BYTES || frame[0] != 1 || !(1..=3).contains(&frame[1]) {
            return None;
        }
        let next = u16_at(frame, 2);
        if self
            .last
            .is_none_or(|last| now.saturating_sub(last) > STALE_AFTER)
        {
            self.sequence = next;
            self.mask = 0;
        } else if next != self.sequence {
            if next.wrapping_sub(self.sequence) as i16 <= 0 {
                return None;
            }
            self.sequence = next;
            self.mask = 0;
        }
        self.last = Some(now);
        let page = (frame[1] - 1) as usize;
        if self.mask & (1 << page) != 0 {
            return None;
        }
        self.data[page * PAGE_BYTES..(page + 1) * PAGE_BYTES].copy_from_slice(frame);
        self.mask |= 1 << page;
        if self.mask == 7 {
            Snapshot::decode(&self.data)
        } else {
            None
        }
    }
    pub fn notification(&mut self, bytes: &[u8], now: Duration) -> Option<Snapshot> {
        // A GATT read returns 60 bytes; firmware notifications use 20 bytes.
        // Keep the same page validation and sequence rules for either form.
        if !matches!(bytes.len(), PAGE_BYTES | SNAPSHOT_BYTES) {
            return None;
        }
        let mut snapshot = None;
        for page in bytes.chunks_exact(PAGE_BYTES) {
            if let Some(complete) = self.accept(page, now) {
                snapshot = Some(complete);
            }
        }
        snapshot
    }
}

/// The original C encoder's fixture, also consumed by the C# client's self-test.
pub fn self_test() -> Result<(), String> {
    let bytes = include_bytes!("../fixtures/snapshot-v1.bin");
    let mut assembler = Assembler::default();
    let now = Duration::from_millis(1);
    if assembler.accept(&bytes[..20], now).is_some()
        || assembler.accept(&bytes[40..], now).is_some()
    {
        return Err("Published an incomplete snapshot".into());
    }
    let snapshot = assembler
        .accept(&bytes[20..40], now)
        .ok_or("Out-of-order assembly failed")?;
    if snapshot != Snapshot::decode(bytes).ok_or("Invalid fixture")?
        || snapshot.current != -250
        || snapshot.power != -120
        || snapshot.speed != 2534
    {
        return Err("Telemetry differs from the original C/C# fixture".into());
    }
    if assembler.accept(&bytes[..20], now).is_some() {
        return Err("Duplicate page was published".into());
    }
    assembler.reset();
    if assembler.accept(&[0; 20], now).is_some() || Snapshot::decode(&bytes[..59]).is_some() {
        return Err("Invalid data accepted".into());
    }
    let page = |index: usize, sequence: u16| {
        let mut frame = bytes[index * 20..(index + 1) * 20].to_vec();
        frame[2..4].copy_from_slice(&sequence.to_le_bytes());
        frame
    };
    assembler.accept(&page(0, 65535), now);
    assembler.accept(&page(1, 65535), now);
    if assembler.accept(&page(2, 65535), now).is_none() {
        return Err("Sequence 65535 failed".into());
    }
    assembler.accept(&page(0, 0), now);
    if assembler.accept(&page(1, 65535), now).is_some() {
        return Err("Old sequence accepted".into());
    }
    assembler.accept(&page(1, 0), now);
    if assembler.accept(&page(2, 0), now).is_none() {
        return Err("Sequence wrap failed".into());
    }
    // A restarted device may begin with an older sequence after a timeout.
    let later = Duration::from_secs(3);
    assembler.accept(&page(0, 65534), later);
    assembler.accept(&page(1, 65534), later);
    if assembler.accept(&page(2, 65534), later).is_none() {
        return Err("Timeout resynchronization failed".into());
    }
    assembler.reset();
    if assembler.notification(bytes, now).is_none() {
        return Err("Full GATT read failed".into());
    }
    let mut mixed = *bytes;
    mixed[22] = mixed[22].wrapping_add(1);
    if Snapshot::decode(&mixed).is_some() {
        return Err("Mixed sequences accepted".into());
    }
    Ok(())
}
