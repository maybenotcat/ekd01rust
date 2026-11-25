//! Application NOR policy and UART diagnostics. The HAL owns only hardware.
use super::DebugUart;
use alloc::rc::Rc;
pub use mt2502_hal::flash::{FlashError, SECTOR_BYTES};
use mt2502_hal::{
    flash::{FlashController, FlashFault, FlashRegion, NOR_BASE, NOR_BYTES, RegionBounds},
    mmio::read32,
    registers::NOR_JEDEC_GD25LQ64,
};

pub const CLAIM: u32 = 0x500000;
pub const STORAGE: u32 = 0x501000;
pub const STORAGE_END: u32 = 0x560000;

/// Settings and identity operations keep absolute NOR offsets for compatibility
/// with existing records. All accesses are bounded to the application data area.
pub struct Flash {
    controller: Rc<FlashController>,
    data: FlashRegion<Rc<FlashController>>,
    writable: bool,
    pub jedec: u32,
}
impl Flash {
    /// Attach the application's single physical NOR owner.
    ///
    /// # Safety
    /// Call once after startup copied the SRAM routines and configured SFI.
    /// No independent owner or raw SFI transaction may coexist with this one.
    pub unsafe fn initialize() -> Self {
        // This is the application's only controller acquisition. All tasks and
        // update confirmation share it; no second attach is allowed.
        let controller = Rc::new(unsafe { FlashController::attach() });
        let jedec = controller.jedec().expect("exclusive flash startup");
        DebugUart::hex("[FLASH] JEDEC", jedec);
        let writable = jedec == NOR_JEDEC_GD25LQ64
            && read32(NOR_BASE + ekd01_boot_contract::ACTIVE.start) & 0xffffff == 0x4d4d4d;
        let bounds = RegionBounds::new(CLAIM, NOR_BYTES).expect("application data bounds");
        Self {
            data: FlashRegion::new(controller.clone(), bounds),
            controller,
            writable,
            jedec,
        }
    }
    pub fn available(&self) -> bool {
        self.writable
    }
    fn relative(offset: u32) -> Result<u32, FlashError> {
        offset.checked_sub(CLAIM).ok_or(FlashError::Range)
    }
    pub fn read(&self, offset: u32, bytes: &mut [u8]) -> Result<(), FlashError> {
        self.data.read(Self::relative(offset)?, bytes)
    }
    pub fn program(&mut self, offset: u32, bytes: &[u8]) -> Result<(), FlashError> {
        let relative = Self::relative(offset)?;
        self.data.bounds().address(relative, bytes.len())?;
        if !self.writable {
            return Err(FlashError::UnknownDevice);
        }
        self.report(self.data.program(relative, bytes))
    }
    pub fn erase(&mut self, offset: u32) -> Result<(), FlashError> {
        let relative = Self::relative(offset)?;
        self.data.bounds().address(relative, SECTOR_BYTES)?;
        if !self.writable {
            return Err(FlashError::UnknownDevice);
        }
        self.report(self.data.erase_sector(relative))
    }
    fn report(&self, result: Result<(), FlashError>) -> Result<(), FlashError> {
        if let Err(error) = result {
            if let Some(fault) = self.data.fault() {
                Self::log_fault(error, fault);
            }
        }
        result
    }
    fn log_fault(error: FlashError, fault: FlashFault) {
        DebugUart::hex("[FLASH] ERROR_KIND", error as u32);
        DebugUart::hex("[FLASH] VERIFY_ADDRESS", fault.address);
        DebugUart::hex(
            "[FLASH] EXPECTED_ACTUAL",
            ((fault.expected as u32) << 8) | fault.actual as u32,
        );
        if let Some(trace) = fault.trace {
            DebugUart::hex("[FLASH] PROGRAM_GPRAM_HEADER", trace.header);
            DebugUart::hex("[FLASH] PROGRAM_GPRAM_DATA", trace.first_data);
            DebugUart::hex("[FLASH] PROGRAM_OUTL", trace.output_bytes);
            DebugUart::hex("[FLASH] PROGRAM_MODE", trace.mode);
            DebugUart::hex("[FLASH] PROGRAM_SR1_AFTER", trace.status_after);
        }
    }
    /// Confirmation uses the same physical controller but its own bounded view.
    pub fn state_region(&self) -> FlashRegion<Rc<FlashController>> {
        let state = ekd01_boot_contract::STATE;
        FlashRegion::new(
            self.controller.clone(),
            RegionBounds::new(state.start, state.end).expect("update state bounds"),
        )
    }
}
