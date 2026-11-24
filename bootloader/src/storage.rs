use crate::soc::{
    feed_watchdog,
    flash::{Flash, FlashError},
};
use ekd01_boot_contract::{ERASE_BYTES, Partition, WRITE_BYTES};
use embedded_storage::nor_flash::{
    ErrorType, NorFlash, NorFlashError, NorFlashErrorKind, ReadNorFlash,
};

impl NorFlashError for FlashError {
    fn kind(&self) -> NorFlashErrorKind {
        match self {
            Self::Range => NorFlashErrorKind::OutOfBounds,
            _ => NorFlashErrorKind::Other,
        }
    }
}

pub struct PartitionFlash {
    region: Partition,
    flash: Flash,
}
impl PartitionFlash {
    pub fn new(region: Partition) -> Self {
        let flash = Flash::partition(region.start, region.end)
            .unwrap_or_else(|_| crate::stop("[BOOT] PARTITION_FAILED\n"));
        Self { region, flash }
    }
}
impl ErrorType for PartitionFlash {
    type Error = FlashError;
}
impl ReadNorFlash for PartitionFlash {
    const READ_SIZE: usize = 1;
    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        let address = self
            .region
            .address(offset, bytes.len())
            .ok_or(FlashError::Range)?;
        feed_watchdog();
        self.flash.read(address, bytes)
    }
    fn capacity(&self) -> usize {
        self.region.len() as usize
    }
}
impl NorFlash for PartitionFlash {
    const WRITE_SIZE: usize = WRITE_BYTES as usize;
    const ERASE_SIZE: usize = ERASE_BYTES as usize;
    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        if offset % WRITE_BYTES != 0 || bytes.len() % Self::WRITE_SIZE != 0 {
            return Err(FlashError::Range);
        }
        let address = self
            .region
            .address(offset, bytes.len())
            .ok_or(FlashError::Range)?;
        feed_watchdog();
        self.flash.program(address, bytes)
    }
    fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        if from > to || from % ERASE_BYTES != 0 || to % ERASE_BYTES != 0 {
            return Err(FlashError::Range);
        }
        self.region
            .address(from, (to - from) as usize)
            .ok_or(FlashError::Range)?;
        for offset in (from..to).step_by(Self::ERASE_SIZE) {
            feed_watchdog();
            self.flash.erase(self.region.start + offset)?;
        }
        Ok(())
    }
}
