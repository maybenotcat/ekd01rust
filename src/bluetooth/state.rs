use portable_atomic::{AtomicBool, AtomicU8, Ordering};
static ENABLED: AtomicBool = AtomicBool::new(false);
static STATUS: AtomicU8 = AtomicU8::new(0);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum BluetoothState {
    Off = 0,
    Starting = 1,
    Ready = 2,
    Connected = 3,
    Stopping = 4,
    Failed = 5,
}
pub struct BluetoothControl;
impl BluetoothControl {
    pub fn set_enabled(enabled: bool) {
        ENABLED.store(enabled, Ordering::Release);
    }
    pub fn enabled() -> bool {
        ENABLED.load(Ordering::Acquire)
    }
    pub fn set_state(state: BluetoothState) {
        STATUS.store(state as u8, Ordering::Release);
    }
    pub fn state() -> BluetoothState {
        match STATUS.load(Ordering::Acquire) {
            1 => BluetoothState::Starting,
            2 => BluetoothState::Ready,
            3 => BluetoothState::Connected,
            4 => BluetoothState::Stopping,
            5 => BluetoothState::Failed,
            _ => BluetoothState::Off,
        }
    }
    pub fn stopped() -> bool {
        matches!(Self::state(), BluetoothState::Off | BluetoothState::Failed)
    }
}
