//! N22 power-on selections and cross-peripheral ordering.
use crate::{memory::RTC_DEFAULTS, power_profile::STARTUP};
use core::marker::PhantomData;
use mt2502_hal::{
    environment::Trace,
    error::Error,
    pmu::{
        control,
        isink::{Channel, CurrentSinks},
    },
    registers::EKD_BOOT_USB_WITHOUT_BATTERY,
    rtc::startup::RtcStartup,
};
pub struct BootPower<P: Trace>(PhantomData<P>);
impl<P: Trace> BootPower<P> {
    pub fn initialize_power() {
        let mut charger = control::Charger;
        charger.set_high_voltage_threshold(11);
        charger.enable_high_voltage_detection();
        let usb = control::boot_flags() & EKD_BOOT_USB_WITHOUT_BATTERY != 0;
        if !usb && charger.input_status() && charger.battery_status_clear() {
            // Original current lookup floors 50000 to 45000 (index 12).
            charger.set_charge_current_index(12);
            charger.enable();
        }
        if !usb {
            control::apply_settings(STARTUP);
        }
        control::enable_adie_interrupts();
        control::set_keypad_led_mode(0);
        control::set_speaker_volume_step(3);
        // Boot is the sole current-sink owner; application acquisition happens later.
        let mut sinks = unsafe { CurrentSinks::claim() };
        sinks.set_mode(0);
        sinks.set_current_step(Channel::Zero, 5);
        sinks.set_current_step(Channel::One, 5);
        control::PowerKey.configure_reset();
        P::log("[BOOT] PMU_READY\n");
    }
    pub fn release_usb_download_power() {
        // The original N22 dispatcher repeats the guarded command twice.
        for _ in 0..2 {
            if control::boot_flags() & EKD_BOOT_USB_WITHOUT_BATTERY == 0 {
                control::Charger.release_usb_download_power();
            }
        }
        P::log("[BOOT] USBDL_POWER_PATH_READY\n");
    }
    pub fn latch_power() -> Result<(), Error> {
        RtcStartup::restore_running_clock()?;
        let state = RtcStartup::power_on_state(RTC_DEFAULTS.alarm)?;
        P::hex("[BOOT] RTC_FIRST_ON", state.first_on as u32);
        P::hex("[BOOT] RTC_CONFIG_VALID", state.configuration_valid as u32);
        if state.first_on || !state.configuration_valid {
            P::log("[BOOT] RTC_FIRST_POWER_ON\n");
            RtcStartup::initialize_first_power_on(
                RTC_DEFAULTS.time,
                |selected_trim, _frequency| {
                    // Original N22 1002cfcc uses SP+0x0c trim, not SP+0x08 frequency.
                    32768 - selected_trim as i32
                },
            )?;
        }
        P::hex("[BOOT] RTC_GPIO", RtcStartup::gpio_control() as u32);
        RtcStartup::enable_power_latch()?;
        RtcStartup::finish_power_on()?;
        P::log("[BOOT] POWER_LATCHED\n");
        Ok(())
    }
}
/// Application power policy after the loader has configured clocks and memory.
pub struct Power<P: Trace> {
    hardware: mt2502_hal::pmu::Pmu,
    _environment: PhantomData<P>,
}
impl<P: Trace> Power<P> {
    pub fn initialize() -> Self {
        P::log("[PMU] INIT_ENTER\n");
        let boot = control::boot_flags();
        let hardware = mt2502_hal::pmu::Pmu::load_calibration();
        critical_section::with(|_| {
            hardware.apply_calibration();
            if boot & EKD_BOOT_USB_WITHOUT_BATTERY == 0 {
                control::apply_settings(STARTUP);
            }
            // PMU startup is before backlight acquisition of the sink bank.
            let mut sinks = unsafe { CurrentSinks::claim() };
            sinks.enable_26mhz_clock();
            control::PowerKey.configure_reset();
        });
        P::hex("[PMU] BOOT_FLAGS", boot);
        P::hex("[PMU] EFUSE", hardware.efuse());
        P::log("[PMU] INIT_EXIT\n");
        Self {
            hardware,
            _environment: PhantomData,
        }
    }
    pub fn enable_lcd_supply(&mut self) {
        self.hardware.enable_vsim1(0);
        P::hex("[PMU] VSIM1_CON0", self.hardware.vsim1_control() as u32);
    }
    pub fn power_pressed(&self) -> bool {
        self.hardware.power_pressed()
    }
}
