//! Bounded views borrow one physical controller; overlapping views are serialized.
use super::{FlashDevice, FlashError, FlashFault, NOR_BYTES, SECTOR_BYTES, ram::operation_ram};
use crate::{
    mmio::write32,
    registers::{WDT_RESTART, WDT_RESTART_KEY},
};
use core::{borrow::Borrow, cell::RefCell};
use embedded_storage::nor_flash::{ErrorType, NorFlash, ReadNorFlash};

/// One owner per physical SFI controller. RefCell deliberately makes it !Sync:
/// firmware tasks share it on one executor, never across threads or IRQ handlers.
pub struct FlashController {
    device: RefCell<FlashDevice>,
}
impl FlashController {
    /// Attach to the configured SFI bus and read JEDEC without changing timings.
    ///
    /// # Safety
    /// The caller must own the physical controller exclusively, have installed
    /// the SRAM routines, and keep clock/pad configuration valid for its lifetime.
    /// All subsequent NOR operations must use this owner. Low-level startup
    /// transactions may precede attachment but must not overlap it.
    pub unsafe fn attach() -> Self {
        let jedec = critical_section::with(|_| unsafe {
            operation_ram(0, 0, core::ptr::null(), 0, core::ptr::null_mut())
        }) as u32;
        Self {
            device: RefCell::new(FlashDevice { jedec, fault: None }),
        }
    }
    pub fn jedec(&self) -> Result<u32, FlashError> {
        Ok(self
            .device
            .try_borrow()
            .map_err(|_| FlashError::Busy)?
            .jedec)
    }
    pub fn fault(&self) -> Option<FlashFault> {
        self.device
            .try_borrow()
            .ok()
            .and_then(|device| device.fault)
    }
}

/// Physical offsets, exclusive end; no board-specific partition policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionBounds {
    start: u32,
    end: u32,
}
impl RegionBounds {
    pub fn new(start: u32, end: u32) -> Result<Self, FlashError> {
        if start >= end
            || end > NOR_BYTES
            || start as usize % SECTOR_BYTES != 0
            || end as usize % SECTOR_BYTES != 0
        {
            return Err(FlashError::Range);
        }
        Ok(Self { start, end })
    }
    pub fn start(self) -> u32 {
        self.start
    }
    pub fn len(self) -> usize {
        (self.end - self.start) as usize
    }
    pub fn address(self, offset: u32, size: usize) -> Result<u32, FlashError> {
        if offset as usize > self.len() || size > self.len() - offset as usize {
            return Err(FlashError::Range);
        }
        Ok(self.start + offset)
    }
}

/// A region holds a borrowed or reference-counted controller owner, not a second
/// peripheral. No borrow guard survives an operation or an async suspension.
#[derive(Clone)]
pub struct FlashRegion<H> {
    controller: H,
    bounds: RegionBounds,
}
impl<H: Borrow<FlashController>> FlashRegion<H> {
    pub fn new(controller: H, bounds: RegionBounds) -> Self {
        Self { controller, bounds }
    }
    pub fn bounds(&self) -> RegionBounds {
        self.bounds
    }
    pub fn fault(&self) -> Option<FlashFault> {
        self.controller.borrow().fault()
    }
    pub fn read(&self, offset: u32, bytes: &mut [u8]) -> Result<(), FlashError> {
        let address = self.bounds.address(offset, bytes.len())?;
        self.controller
            .borrow()
            .device
            .try_borrow()
            .map_err(|_| FlashError::Busy)?
            .read(address, bytes)
    }
    /// Byte programming for filesystem and identity records.
    pub fn program(&self, offset: u32, bytes: &[u8]) -> Result<(), FlashError> {
        let address = self.bounds.address(offset, bytes.len())?;
        self.controller
            .borrow()
            .device
            .try_borrow_mut()
            .map_err(|_| FlashError::Busy)?
            .program(address, bytes)
    }
    pub fn erase_sector(&self, offset: u32) -> Result<(), FlashError> {
        let address = self.bounds.address(offset, SECTOR_BYTES)?;
        self.controller
            .borrow()
            .device
            .try_borrow_mut()
            .map_err(|_| FlashError::Busy)?
            .erase(address)
    }
}
impl<H: Borrow<FlashController>> ErrorType for FlashRegion<H> {
    type Error = FlashError;
}
impl<H: Borrow<FlashController>> ReadNorFlash for FlashRegion<H> {
    const READ_SIZE: usize = 1;
    fn capacity(&self) -> usize {
        self.bounds.len()
    }
    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), FlashError> {
        self.bounds.address(offset, bytes.len())?;
        write32(WDT_RESTART, WDT_RESTART_KEY);
        FlashRegion::read(self, offset, bytes)
    }
}
impl<H: Borrow<FlashController>> NorFlash for FlashRegion<H> {
    const WRITE_SIZE: usize = 4;
    const ERASE_SIZE: usize = SECTOR_BYTES;
    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), FlashError> {
        if offset % Self::WRITE_SIZE as u32 != 0 || bytes.len() % Self::WRITE_SIZE != 0 {
            return Err(FlashError::Range);
        }
        self.bounds.address(offset, bytes.len())?;
        write32(WDT_RESTART, WDT_RESTART_KEY);
        self.program(offset, bytes)
    }
    fn erase(&mut self, from: u32, to: u32) -> Result<(), FlashError> {
        if from > to || from as usize % SECTOR_BYTES != 0 || to as usize % SECTOR_BYTES != 0 {
            return Err(FlashError::Range);
        }
        self.bounds.address(from, (to - from) as usize)?;
        for offset in (from..to).step_by(SECTOR_BYTES) {
            write32(WDT_RESTART, WDT_RESTART_KEY);
            self.erase_sector(offset)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn controller() -> FlashController {
        FlashController {
            device: RefCell::new(FlashDevice {
                jedec: 0,
                fault: None,
            }),
        }
    }
    #[test]
    fn bounds_reject_escape_overflow_and_unaligned_regions() {
        for (start, end) in [
            (1, 4096),
            (0, 4097),
            (4096, 4096),
            (4096, 0),
            (0, NOR_BYTES + 4096),
        ] {
            assert_eq!(RegionBounds::new(start, end), Err(FlashError::Range));
        }
        let bounds = RegionBounds::new(4096, 8192).unwrap();
        assert_eq!(bounds.address(0, 4096), Ok(4096));
        assert_eq!(bounds.address(4096, 0), Ok(8192));
        for (offset, size) in [(4096, 1), (4097, 0), (1, usize::MAX), (u32::MAX, 1)] {
            assert_eq!(bounds.address(offset, size), Err(FlashError::Range));
        }
    }
    #[test]
    fn shared_regions_report_busy_without_panicking_or_accessing_mmio() {
        let controller = controller();
        let bounds = RegionBounds::new(0, 4096).unwrap();
        let a = FlashRegion::new(&controller, bounds);
        let b = a.clone();
        let guard = controller.device.borrow_mut();
        assert_eq!(a.read(0, &mut []), Err(FlashError::Busy));
        assert_eq!(b.program(0, &[]), Err(FlashError::Busy));
        assert_eq!(b.erase_sector(0), Err(FlashError::Busy));
        drop(guard);
        assert_eq!(a.read(0, &mut []), Ok(()));
        assert_eq!(a.program(0, &[]), Err(FlashError::UnknownDevice));
        assert_eq!(a.read(4096, &mut [0]), Err(FlashError::Range));
        assert_eq!(b.erase_sector(4096), Err(FlashError::Range));
    }
    #[test]
    fn storage_traits_reject_invalid_requests_before_touching_hardware() {
        let controller = controller();
        let mut region = FlashRegion::new(&controller, RegionBounds::new(0, 4096).unwrap());
        assert_eq!(ReadNorFlash::capacity(&region), 4096);
        assert_eq!(
            ReadNorFlash::read(&mut region, 4096, &mut [0]),
            Err(FlashError::Range)
        );
        for (offset, bytes) in [(1, &[][..]), (0, &[0][..]), (4096, &[0; 4][..])] {
            assert_eq!(
                NorFlash::write(&mut region, offset, bytes),
                Err(FlashError::Range)
            );
        }
        for (from, to) in [(1, 4096), (0, 4095), (4096, 0), (0, 8192)] {
            assert_eq!(
                NorFlash::erase(&mut region, from, to),
                Err(FlashError::Range)
            );
        }
    }
}
