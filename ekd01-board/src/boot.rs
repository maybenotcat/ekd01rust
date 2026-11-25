//! Explicit EKD01 N22 startup sequence; low-level failures are returned to the loader.
use crate::{error::BoardError, memory, power::BootPower};
use core::marker::PhantomData;
use mt2502_hal::{
    cache, clocks::Clocks, debug_uart::Uart1, environment::Trace, memory_map, psram,
    rtc::startup::RtcStartup, watchdog::Watchdog,
};
pub struct Boot<P: Trace>(PhantomData<P>);
impl<P: Trace> Boot<P> {
    pub fn initialize_clocks(usb_boot: bool) -> Result<(), BoardError> {
        memory_map::set_boot_control(2);
        Clocks::initialize_clocks(usb_boot, memory::CLOCKS)?;
        Ok(())
    }
    pub fn initialize_uart() {
        Uart1::initialize(28);
    }
    /// # Safety
    /// Execute in the SRAM-linked loader before tasks/IRQs and application RAM use.
    pub unsafe fn initialize_platform() -> Result<(), BoardError> {
        memory::initialize_emi::<P>()?;
        RtcStartup::initialize_oscillator()?;
        Clocks::stabilize_32k()?;
        P::log("[BOOT] 32K_STABLE\n");
        P::hex("[BOOT] EMI_GENA", mt2502_hal::emi::configuration_word());
        if !psram::probe_locations(memory::PSRAM_PROBES) {
            return Err(BoardError::PsramProbeFailed);
        }
        P::log("[BOOT] PSRAM_OK\n");
        cache::invalidate_and_disable();
        // The loader executes from SRAM with masked interrupts.
        unsafe {
            memory_map::install_vectors(memory::BOOT_VECTOR_SOURCE);
        }
        unsafe {
            memory::initialize_flash::<P>()?;
        }
        BootPower::<P>::initialize_power();
        BootPower::<P>::release_usb_download_power();
        BootPower::<P>::latch_power()?;
        Ok(())
    }
    pub fn finish_handoff() {
        cache::disable_for_jump();
        Uart1::wait_transmitter_empty_feeding_watchdog();
        memory_map::set_boot_control(3);
    }
    pub fn arm_application_watchdog() {
        Watchdog::arm_application();
    }
}
