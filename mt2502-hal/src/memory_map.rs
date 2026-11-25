//! Boot mapping control and low vectors, independent of the executable's layout.
use crate::mmio::*;
pub fn set_boot_control(control: u16) {
    write16(0xa0510000, control);
}
/// # Safety
/// The source must contain eight ARM absolute vectors (64 readable bytes).
/// The caller must own exception routing and have masked interrupts.
pub unsafe fn install_vectors(source: u32) {
    for offset in (0..64).step_by(4) {
        let word = read32(source + offset);
        // Physical vector zero is valid hardware memory, not a Rust pointer.
        write_physical_word(offset, word);
    }
}
