//! GD25LQ64C driver. Manual SFI transactions must execute entirely from SRAM.
use self::geometry::{PROGRAM_DATA_MAX, program_chunk};
use crate::{DebugUart, mmio::*, registers::*};

pub const CLAIM: u32 = 0x500000;
pub const STORAGE: u32 = 0x501000;
pub const STORAGE_END: u32 = 0x560000;
pub const SECTOR_BYTES: usize = 4096;
const NOR_BASE: u32 = 0x10000000;
const NOR_BYTES: u32 = 0x800000;

#[derive(Default)]
#[repr(C)]
struct ProgramTrace {
    header: u32,
    first_data: u32,
    output_bytes: u32,
    mode: u32,
    status_after: u32,
}
impl ProgramTrace {
    fn report(&self) {
        DebugUart::hex("[FLASH] PROGRAM_GPRAM_HEADER", self.header);
        DebugUart::hex("[FLASH] PROGRAM_GPRAM_DATA", self.first_data);
        DebugUart::hex("[FLASH] PROGRAM_OUTL", self.output_bytes);
        DebugUart::hex("[FLASH] PROGRAM_MODE", self.mode);
        DebugUart::hex("[FLASH] PROGRAM_SR1_AFTER", self.status_after);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlashError {
    Range,
    UnknownDevice,
    NeedsErase,
    Protected,
    Verify,
    Suspended,
}
pub struct Flash {
    writable: bool,
    write_start: u32,
    write_end: u32,
    pub jedec: u32,
}
impl Flash {
    pub fn initialize() -> Self {
        let jedec = critical_section::with(|_| unsafe {
            operation_ram(0, 0, core::ptr::null(), 0, core::ptr::null_mut())
        }) as u32;
        let magic = read32(NOR_BASE + 0x50000) & 0xffffff;
        DebugUart::hex("[FLASH] JEDEC", jedec);
        let writable = jedec == NOR_JEDEC_GD25LQ64 && magic == 0x4d4d4d;
        Self {
            writable,
            write_start: CLAIM,
            write_end: NOR_BYTES,
            jedec,
        }
    }
    /// Boot/update code must explicitly select its owned partition. Ordinary
    /// settings operations retain the original protected write window.
    pub fn partition(start: u32, end: u32) -> Result<Self, FlashError> {
        if start < 0x050000 || start >= end || end > CLAIM || start & 4095 != 0 || end & 4095 != 0 {
            return Err(FlashError::Range);
        }
        let mut flash = Self::initialize();
        // During an interrupted swap the active GFH may be erased. Boot
        // recovery must depend on the detected NOR, not on a valid active image.
        flash.writable = flash.jedec == NOR_JEDEC_GD25LQ64;
        flash.write_start = start;
        flash.write_end = end;
        Ok(flash)
    }
    pub fn available(&self) -> bool {
        self.writable
    }
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
        if offset < self.write_start
            || offset > self.write_end
            || size > (self.write_end - offset) as usize
        {
            return Err(FlashError::Range);
        }
        if !self.writable {
            return Err(FlashError::UnknownDevice);
        }
        Ok(())
    }
    pub fn program(&mut self, offset: u32, bytes: &[u8]) -> Result<(), FlashError> {
        self.check_write(offset, bytes.len())?;
        if let Some((index, &wanted)) = bytes
            .iter()
            .enumerate()
            .find(|(i, b)| read8(NOR_BASE + offset + *i as u32) & **b != **b)
        {
            DebugUart::hex("[FLASH] NEEDS_ERASE_ADDRESS", offset + index as u32);
            DebugUart::hex(
                "[FLASH] EXISTING_REQUESTED",
                ((read8(NOR_BASE + offset + index as u32) as u32) << 8) | wanted as u32,
            );
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
                    DebugUart::hex("[FLASH] VERIFY_ADDRESS", address + i as u32);
                    DebugUart::hex(
                        "[FLASH] EXPECTED_ACTUAL",
                        ((page[i] as u32) << 8) | read8(NOR_BASE + address + i as u32) as u32,
                    );
                    trace.report();
                    return Err(FlashError::Verify);
                }
            }
            written += count;
        }
        Ok(())
    }
    pub fn erase(&mut self, offset: u32) -> Result<(), FlashError> {
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
            DebugUart::hex("[FLASH] ERASE_VERIFY_ADDRESS", offset + index as u32);
            DebugUart::hex(
                "[FLASH] ERASE_VERIFY_ACTUAL",
                read8(NOR_BASE + offset + index as u32) as u32,
            );
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

impl embedded_storage::nor_flash::NorFlashError for FlashError {
    fn kind(&self) -> embedded_storage::nor_flash::NorFlashErrorKind {
        match self {
            Self::Range => embedded_storage::nor_flash::NorFlashErrorKind::OutOfBounds,
            _ => embedded_storage::nor_flash::NorFlashErrorKind::Other,
        }
    }
}
