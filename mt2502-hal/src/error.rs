//! Hardware operation failures; callers choose recovery and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    SfiUnsupportedDevice,
    SfiPllUnsupported,
    PowerLatchFailed,
    Clock32kBusy,
    Clock32kUnstable,
    EmiCalibrationFailed,
    EmiTimeout { register: u32 },
    FqmtrBusy,
    FqmtrZero,
    GptTimeout,
    RtcAlarmInitFailed,
    RtcBusy,
    RtcFirstPowerOnFailed,
    RtcNo32k,
}
