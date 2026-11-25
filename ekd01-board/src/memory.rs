//! N22 clock/memory profiles and BootROM metadata locations.
use mt2502_hal::{clocks::Configuration, emi::TimingSource, mmio::read32};

pub const CLOCKS: Configuration = Configuration {
    mpll_control: 0x1401,
    mpll_auxiliary: 0,
    domain_h: 0xd001,
    domain_c: 0xb620,
};
pub const EMI_ARBITRATION: [u32; 3] = [0x5431, 0x5009, 0x5046];
pub const PSRAM_PROBES: [u32; 3] = [0x100000, 0x300000, 0x6efffc];
pub const BOOT_VECTOR_SOURCE: u32 = 0x70006140;

/// GFH EMI_LIST v5 loaded into SRAM by BootROM. Keep lazy reads in register order.
pub struct BootRomTimings;
impl TimingSource for BootRomTimings {
    fn word(&self, index: u32) -> u32 {
        read32(0x700060dc + index * 4)
    }
}
pub const RTC_DEFAULTS: mt2502_hal::rtc::startup::Defaults = mt2502_hal::rtc::startup::Defaults {
    time: [0, 0, 0, 1, 1, 1, 21],
    alarm: [0, 0, 0, 1, 1, 0],
};
pub const FLASH: mt2502_hal::sfi::Configuration = mt2502_hal::sfi::Configuration {
    jedec: crate::flash_profile::JEDEC,
    commands: crate::flash_profile::DEVICE_COMMANDS,
    hardware: crate::flash_profile::HW_CONF_130M,
};

/// Configure the board's NOR, then validate its BootROM-loaded header mirror.
/// # Safety
/// The loader and all configuration data must already reside in SRAM; SFI must
/// be exclusively owned and no concurrent NOR access may occur.
pub unsafe fn initialize_flash<P: mt2502_hal::environment::Trace>()
-> Result<(), crate::error::BoardError> {
    use mt2502_hal::{clocks::Clocks, registers::*, sfi::Sfi};
    P::log("[BOOT] SFI_INIT_ENTER\n");
    Clocks::initialize_bus_pads();
    // This function is invoked by the SRAM-linked loader before application handoff.
    let jedec = unsafe { mt2502_hal::flash::probe_jedec() };
    P::hex("[FLASH] JEDEC", jedec);
    unsafe {
        Sfi::configure(&FLASH, jedec)?;
    }
    for offset in (0..0x140).step_by(4) {
        if read32(EKD_NOR_BASE + 0x800 + offset) != read32(0x70006000 + offset) {
            P::hex("[BOOT] SFI_READBACK_OFFSET", offset);
            return Err(crate::error::BoardError::BootHeaderMismatch { offset });
        }
    }
    P::hex("[BOOT] SFI_DIRECT_CTL", mt2502_hal::sfi::direct_control());
    P::hex("[BOOT] CLK_CONDC", mt2502_hal::clocks::domain_c_control());
    P::log("[BOOT] SFI_INIT_EXIT\n");
    Ok(())
}
pub struct Memory<P: mt2502_hal::environment::Trace>(core::marker::PhantomData<P>);
impl<P: mt2502_hal::environment::Trace> Memory<P> {
    /// # Safety
    /// The linker-reserved range must be unused initialized PSRAM.
    pub unsafe fn probe_heap(begin: u32, end: u32) -> bool {
        unsafe { mt2502_hal::psram::Psram::<P>::probe_heap(begin, end) }
    }
    pub fn initialize_cache(image_end: u32) {
        let end = (image_end + 4095) & !4095;
        // This board image executes in the existing NOR mapping; entry copied
        // the cache routine into SRAM before invoking this policy.
        critical_section::with(|_| unsafe {
            mt2502_hal::cache::configure_xip(0x10050180, end);
        });
        mt2502_hal::watchdog::Watchdog::feed();
        P::hex("[CACHE] CON", mt2502_hal::cache::control() as u32);
        P::log("[CACHE] XIP_32K_4WAY; uncached physical PSRAM\n");
    }
}

pub fn initialize_emi<P: mt2502_hal::environment::Trace>() -> Result<(), crate::error::BoardError> {
    P::log("[BOOT] EMI_INIT_ENTER\n");
    let result = mt2502_hal::emi::Emi::initialize(&BootRomTimings, EMI_ARBITRATION);
    if let Err(mt2502_hal::error::Error::EmiTimeout { register }) = result {
        P::hex("[BOOT] EMI_TIMEOUT_REGISTER", register);
    }
    result?;
    P::log("[BOOT] EMI_INIT_EXIT\n");
    Ok(())
}
