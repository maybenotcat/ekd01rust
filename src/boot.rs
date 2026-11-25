//! Confirm a trial only after the scheduler, UI renderer and LCD DMA work.
use crate::soc::{
    DebugUart, Watchdog,
    flash::{Flash, FlashError},
    mmio::{read32, write32},
};
use ekd01_boot_contract as layout;
use embedded_storage::nor_flash::{
    ErrorType, NorFlash, ReadNorFlash,
};

struct StateFlash(Flash);

/// Capture the reset-time register handoff instead of trusting retained PSRAM
/// after a warm reset through a different/legacy loader.
pub fn capture_handoff(magic: u32, inverse: u32, version: u32, trial: u32) {
    let valid = magic == layout::HANDOFF_MAGIC
        && inverse == !magic
        && version == layout::HANDOFF_VERSION
        && trial <= 1;
    write32(layout::HANDOFF_ADDRESS, if valid { magic } else { 0 });
    write32(layout::HANDOFF_ADDRESS + 4, inverse);
    write32(layout::HANDOFF_ADDRESS + 8, version);
    write32(layout::HANDOFF_ADDRESS + 12, trial);
}
impl ErrorType for StateFlash {
    type Error = FlashError;
}
impl ReadNorFlash for StateFlash {
    const READ_SIZE: usize = 1;
    fn capacity(&self) -> usize {
        layout::STATE.len() as usize
    }
    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), FlashError> {
        self.0.read(
            layout::STATE
                .address(offset, bytes.len())
                .ok_or(FlashError::Range)?,
            bytes,
        )
    }
}
impl NorFlash for StateFlash {
    const WRITE_SIZE: usize = layout::WRITE_BYTES as usize;
    const ERASE_SIZE: usize = layout::ERASE_BYTES as usize;
    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), FlashError> {
        if offset & 3 != 0 || bytes.len() & 3 != 0 {
            return Err(FlashError::Range);
        }
        self.0.program(
            layout::STATE
                .address(offset, bytes.len())
                .ok_or(FlashError::Range)?,
            bytes,
        )
    }
    fn erase(&mut self, from: u32, to: u32) -> Result<(), FlashError> {
        if from > to || from & 4095 != 0 || to & 4095 != 0 {
            return Err(FlashError::Range);
        }
        layout::STATE
            .address(from, (to - from) as usize)
            .ok_or(FlashError::Range)?;
        for offset in (from..to).step_by(Self::ERASE_SIZE) {
            Watchdog::feed();
            self.0.erase(layout::STATE.start + offset)?;
        }
        Ok(())
    }
}

pub fn confirm_first_frame() {
    if read32(layout::HANDOFF_ADDRESS) != layout::HANDOFF_MAGIC
        || read32(layout::HANDOFF_ADDRESS + 4) != !layout::HANDOFF_MAGIC
        || read32(layout::HANDOFF_ADDRESS + 8) != layout::HANDOFF_VERSION
    {
        DebugUart::log("[BOOT] LEGACY_HANDOFF; no state writes\n");
        return;
    }
    if read32(layout::HANDOFF_ADDRESS + 12) == 0 {
        return;
    }
    let Ok(flash) = Flash::partition(layout::STATE.start, layout::STATE.end) else {
        DebugUart::log("[BOOT] CONFIRM_PARTITION_FAILED\n");
        return;
    };
    let mut buffer = embassy_boot::AlignedBuffer([0u8; 4]);
    let mut state = embassy_boot::BlockingFirmwareState::new(StateFlash(flash), &mut buffer.0);
    if state.mark_booted().is_ok() {
        write32(layout::HANDOFF_ADDRESS + 12, 0);
        DebugUart::log("[BOOT] TRIAL_CONFIRMED\n");
    } else {
        DebugUart::log("[BOOT] TRIAL_CONFIRM_FAILED; next reset reverts\n");
    }
}
