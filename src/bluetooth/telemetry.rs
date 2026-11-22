use crate::motor::client::BikeStatus;
use core::cell::RefCell;
use critical_section::Mutex;

static FRAME: Mutex<RefCell<[u8; 60]>> = Mutex::new(RefCell::new([0; 60]));
pub struct TelemetryService;
impl TelemetryService {
    fn put16(out: &mut [u8], offset: usize, value: u16) {
        out[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    fn put32(out: &mut [u8], offset: usize, value: u32) {
        out[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    pub fn encode(status: &BikeStatus, now: u32, sequence: u16) -> [u8; 60] {
        let mut out = [0; 60];
        for page in 0..3 {
            let offset = page * 20;
            out[offset] = 1;
            out[offset + 1] = page as u8 + 1;
            Self::put16(&mut out, offset + 2, sequence);
        }
        let t = &status.telemetry;
        let ride = t.ride_fresh(now);
        let electric = t.electric_fresh(now);
        let adc = !electric && status.voltage_fresh(now);
        let voltage = if electric {
            t.voltage as u32
        } else if adc {
            status.mcu_voltage_mv / 10
        } else {
            0
        };
        let power = if electric {
            (t.voltage as i32 * t.current as i32 / 10000).clamp(-32768, 32767)
        } else {
            0
        };
        Self::put32(&mut out, 4, now);
        Self::put16(&mut out, 8, t.speed);
        Self::put16(&mut out, 10, voltage.min(65535) as u16);
        Self::put16(&mut out, 12, t.current as u16);
        Self::put16(&mut out, 14, power as u16);
        out[16] = t.cadence;
        out[17] = if t.battery <= 100 { t.battery } else { 255 };
        out[18] = status.assist;
        out[19] = ride as u8
            | ((electric as u8) << 1)
            | ((adc as u8) << 2)
            | ((status.active as u8) << 3)
            | ((status.lights as u8) << 4)
            | ((status.can_online(now) as u8) << 5);
        Self::put32(&mut out, 24, t.trip_m);
        Self::put16(&mut out, 28, t.range);
        Self::put16(&mut out, 30, t.controller_c as u16);
        Self::put16(&mut out, 32, t.motor_c as u16);
        out[34] = status.levels;
        Self::put32(&mut out, 36, status.rx);
        Self::put32(&mut out, 44, status.tx);
        Self::put32(&mut out, 48, status.uart_errors);
        Self::put32(&mut out, 52, status.malformed);
        Self::put32(&mut out, 56, status.mcu_rx);
        out
    }
    pub fn publish(status: &BikeStatus, now: u32, sequence: u16) {
        let frame = Self::encode(status, now, sequence);
        critical_section::with(|cs| *FRAME.borrow(cs).borrow_mut() = frame);
    }
    pub fn snapshot() -> [u8; 60] {
        critical_section::with(|cs| *FRAME.borrow(cs).borrow())
    }
}
