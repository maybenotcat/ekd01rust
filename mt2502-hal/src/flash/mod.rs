//! GD25LQ64C driver. Manual SFI transactions must execute entirely from SRAM.
use self::geometry::{PROGRAM_DATA_MAX, program_chunk};
use crate::{mmio::*, registers::*};

pub const SECTOR_BYTES: usize = 4096;
pub const NOR_BASE: u32 = 0x10000000;
pub const NOR_BYTES: u32 = 0x800000;

#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub struct ProgramTrace {
    pub header: u32,
    pub first_data: u32,
    pub output_bytes: u32,
    pub mode: u32,
    pub status_after: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlashError {
    Range,
    UnknownDevice,
    NeedsErase,
    Protected,
    Verify,
    Suspended,
    Busy,
}
/// Diagnostic data captured only after mapped NOR access is restored.
#[derive(Debug, Clone, Copy)]
pub struct FlashFault {
    pub address: u32,
    pub expected: u8,
    pub actual: u8,
    pub trace: Option<ProgramTrace>,
}

/// Physical device, kept private behind the single controller.
struct FlashDevice {
    jedec: u32,
    fault: Option<FlashFault>,
}
impl FlashDevice {
    pub fn read(&self, offset: u32, output: &mut [u8]) -> Result<(), FlashError> {
        if offset > NOR_BYTES || output.len() > (NOR_BYTES - offset) as usize {
            return Err(FlashError::Range);
        }
        for (index, byte) in output.iter_mut().enumerate() {
            *byte = read8(NOR_BASE + offset + index as u32);
        }
        Ok(())
    }
    fn check_write(&self, offset: u32, size: usize) -> Result<(), FlashError> {
        if offset > NOR_BYTES || size > (NOR_BYTES - offset) as usize {
            return Err(FlashError::Range);
        }
        if self.jedec != NOR_JEDEC_GD25LQ64 {
            return Err(FlashError::UnknownDevice);
        }
        Ok(())
    }
    pub fn program(&mut self, offset: u32, bytes: &[u8]) -> Result<(), FlashError> {
        self.fault = None;
        self.check_write(offset, bytes.len())?;
        if let Some((index, &wanted)) = bytes
            .iter()
            .enumerate()
            .find(|(i, b)| read8(NOR_BASE + offset + *i as u32) & **b != **b)
        {
            self.fault = Some(FlashFault {
                address: offset + index as u32,
                expected: wanted,
                actual: read8(NOR_BASE + offset + index as u32),
                trace: None,
            });
            return Err(FlashError::NeedsErase);
        }
        let mut written = 0;
        while written < bytes.len() {
            let address = offset + written as u32;
            let count = program_chunk(address, bytes.len() - written);
            let mut page = [0u8; PROGRAM_DATA_MAX];
            page[..count].copy_from_slice(&bytes[written..written + count]);
            let mut trace = ProgramTrace::default();
            let result = critical_section::with(|_| unsafe {
                operation_ram(1, address, page.as_ptr(), count, &mut trace)
            });
            Self::result(result)?;
            for i in 0..count {
                if read8(NOR_BASE + address + i as u32) != page[i] {
                    self.fault = Some(FlashFault {
                        address: address + i as u32,
                        expected: page[i],
                        actual: read8(NOR_BASE + address + i as u32),
                        trace: Some(trace),
                    });
                    return Err(FlashError::Verify);
                }
            }
            written += count;
        }
        Ok(())
    }
    pub fn erase(&mut self, offset: u32) -> Result<(), FlashError> {
        self.fault = None;
        self.check_write(offset, SECTOR_BYTES)?;
        if offset & 4095 != 0 {
            return Err(FlashError::Range);
        }
        let result = critical_section::with(|_| unsafe {
            operation_ram(2, offset, core::ptr::null(), 0, core::ptr::null_mut())
        });
        Self::result(result)?;
        if let Some(index) = (0..SECTOR_BYTES).find(|i| read8(NOR_BASE + offset + *i as u32) != 255)
        {
            self.fault = Some(FlashFault {
                address: offset + index as u32,
                expected: 255,
                actual: read8(NOR_BASE + offset + index as u32),
                trace: None,
            });
            return Err(FlashError::Verify);
        }
        Ok(())
    }
    fn result(result: i32) -> Result<(), FlashError> {
        if result == 0 {
            Ok(())
        } else if result == -6 {
            Err(FlashError::Suspended)
        } else {
            Err(FlashError::Protected)
        }
    }
}

pub mod geometry;
mod ram;
use ram::operation_ram;
pub use ram::transfer_ram;

/// Read JEDEC during platform startup, before attaching the runtime owner.
///
/// # Safety
/// The caller must exclusively own SFI and have installed the SRAM routines.
/// No region/controller operation may overlap this low-level transaction.
pub unsafe fn probe_jedec() -> u32 {
    critical_section::with(|_| unsafe {
        operation_ram(0, 0, core::ptr::null(), 0, core::ptr::null_mut())
    }) as u32
}

impl embedded_storage::nor_flash::NorFlashError for FlashError {
    fn kind(&self) -> embedded_storage::nor_flash::NorFlashErrorKind {
        match self {
            Self::Range => embedded_storage::nor_flash::NorFlashErrorKind::OutOfBounds,
            _ => embedded_storage::nor_flash::NorFlashErrorKind::Other,
        }
    }
}

mod region;
pub use region::{FlashController, FlashRegion, RegionBounds};
