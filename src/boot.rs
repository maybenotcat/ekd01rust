//! Confirm a trial only after the scheduler, UI renderer and LCD DMA work.
use crate::{
    soc::{
        DebugUart,
        mmio::{read32, write32},
    },
    storage::FlashBus,
};
use ekd01_boot_contract as layout;

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
pub fn confirm_first_frame(bus: &FlashBus) {
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
    let flash = bus.borrow().state_region();
    let mut buffer = embassy_boot::AlignedBuffer([0u8; 4]);
    let mut state = embassy_boot::BlockingFirmwareState::new(flash, &mut buffer.0);
    if state.mark_booted().is_ok() {
        write32(layout::HANDOFF_ADDRESS + 12, 0);
        DebugUart::log("[BOOT] TRIAL_CONFIRMED\n");
    } else {
        DebugUart::log("[BOOT] TRIAL_CONFIRM_FAILED; next reset reverts\n");
    }
}
