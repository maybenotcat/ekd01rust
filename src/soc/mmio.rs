//! MMIO widths and write-only aliases are explicit at each call site.
use core::ptr::{read_volatile, write_volatile};
#[inline(always)]
pub fn read8(address: u32) -> u8 {
    unsafe { read_volatile(address as *const u8) }
}
#[inline(always)]
pub fn read16(address: u32) -> u16 {
    unsafe { read_volatile(address as *const u16) }
}
#[inline(always)]
pub fn read32(address: u32) -> u32 {
    unsafe { read_volatile(address as *const u32) }
}
#[inline(always)]
pub fn write8(address: u32, value: u8) {
    unsafe {
        write_volatile(address as *mut u8, value);
    }
}
#[inline(always)]
pub fn write16(address: u32, value: u16) {
    unsafe {
        write_volatile(address as *mut u16, value);
    }
}
#[inline(always)]
pub fn write32(address: u32, value: u32) {
    unsafe {
        write_volatile(address as *mut u32, value);
    }
}
#[inline(always)]
pub fn update16(address: u32, clear: u32, set: u32) {
    write16(address, (read16(address) & !(clear as u16)) | set as u16);
}
#[inline(always)]
pub fn update32(address: u32, clear: u32, set: u32) {
    write32(address, (read32(address) & !clear) | set);
}
