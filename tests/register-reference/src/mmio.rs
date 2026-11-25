//! Frozen RMW semantics; only primitive bus access is redirected to the native script.
pub use mt2502_hal::mmio::{read8, read16, read32, write8, write16, write32, write_physical_word};
pub fn update16(address: u32, clear: u32, set: u32) {
    write16(address, (read16(address) & !(clear as u16)) | set as u16);
}
pub fn update32(address: u32, clear: u32, set: u32) {
    write32(address, (read32(address) & !clear) | set);
}