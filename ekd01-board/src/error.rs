use mt2502_hal::error::Error;
fn hardware_message(error: Error) -> &'static str {
    match error {
        Error::SfiUnsupportedDevice => "[BOOT] SFI_UNSUPPORTED_DEVICE\n",
        Error::SfiPllUnsupported => "[BOOT] SFI_PLL_UNSUPPORTED\n",
        Error::PowerLatchFailed => "[BOOT] POWER_LATCH_FAILED\n",
        Error::Clock32kBusy => "[BOOT] 32K_BUSY\n",
        Error::Clock32kUnstable => "[BOOT] 32K_UNSTABLE\n",
        Error::EmiCalibrationFailed => "[BOOT] EMI_CALIBRATION_FAILED\n",
        Error::EmiTimeout { .. } => "[BOOT] EMI_TIMEOUT\n",
        Error::FqmtrBusy => "[BOOT] FQMTR_BUSY\n",
        Error::FqmtrZero => "[BOOT] FQMTR_ZERO\n",
        Error::GptTimeout => "[BOOT] GPT_TIMEOUT\n",
        Error::RtcAlarmInitFailed => "[BOOT] RTC_ALARM_INIT_FAILED\n",
        Error::RtcBusy => "[BOOT] RTC_BUSY\n",
        Error::RtcFirstPowerOnFailed => "[BOOT] RTC_FIRST_POWER_ON_FAILED\n",
        Error::RtcNo32k => "[BOOT] RTC_NO_32K\n",
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardError {
    Hardware(Error),
    BootHeaderMismatch { offset: u32 },
    PsramProbeFailed,
}
impl From<Error> for BoardError {
    fn from(error: Error) -> Self {
        Self::Hardware(error)
    }
}
pub fn boot_message(error: BoardError) -> &'static str {
    match error {
        BoardError::Hardware(error) => hardware_message(error),
        BoardError::BootHeaderMismatch { .. } => "[BOOT] SFI_READBACK_FAILED\n",
        BoardError::PsramProbeFailed => "[BOOT] PSRAM_PROBE_FAILED\n",
    }
}
