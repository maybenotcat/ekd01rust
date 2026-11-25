//! Explicit register widths with a statically selected production/test backend.
use core::ptr::{read_volatile, write_volatile};

pub trait RegisterIo {
    fn read8(address: u32) -> u8;
    fn read16(address: u32) -> u16;
    fn read32(address: u32) -> u32;
    fn write8(address: u32, value: u8);
    fn write16(address: u32, value: u16);
    fn write32(address: u32, value: u32);
    fn write_physical_word(address: u32, value: u32) {
        Self::write32(address, value);
    }
}
pub struct VolatileIo;
impl RegisterIo for VolatileIo {
    #[inline(always)]
    fn write_physical_word(address: u32, value: u32) {
        #[cfg(target_arch = "arm")]
        unsafe {
            core::arch::asm!("str {value}, [{address}]", value=in(reg) value,
            address=in(reg) address, options(nostack, preserves_flags));
        }
        #[cfg(not(target_arch = "arm"))]
        Self::write32(address, value);
    }
    #[inline(always)]
    fn read8(address: u32) -> u8 {
        unsafe { read_volatile(address as *const u8) }
    }
    #[inline(always)]
    fn read16(address: u32) -> u16 {
        unsafe { read_volatile(address as *const u16) }
    }
    #[inline(always)]
    fn read32(address: u32) -> u32 {
        unsafe { read_volatile(address as *const u32) }
    }
    #[inline(always)]
    fn write8(address: u32, value: u8) {
        unsafe { write_volatile(address as *mut u8, value) }
    }
    #[inline(always)]
    fn write16(address: u32, value: u16) {
        unsafe { write_volatile(address as *mut u16, value) }
    }
    #[inline(always)]
    fn write32(address: u32, value: u32) {
        unsafe { write_volatile(address as *mut u32, value) }
    }
}
#[cfg(not(feature = "register-tests"))]
type Bus = VolatileIo;
#[cfg(feature = "register-tests")]
type Bus = crate::testing::ScriptIo;

#[inline(always)]
pub fn read8(address: u32) -> u8 {
    Bus::read8(address)
}
#[inline(always)]
pub fn read16(address: u32) -> u16 {
    Bus::read16(address)
}
#[inline(always)]
pub fn read32(address: u32) -> u32 {
    Bus::read32(address)
}
#[inline(always)]
pub fn write8(address: u32, value: u8) {
    Bus::write8(address, value);
}
#[inline(always)]
pub fn write16(address: u32, value: u16) {
    Bus::write16(address, value);
}
#[inline(always)]
pub fn write32(address: u32, value: u32) {
    Bus::write32(address, value);
}
#[inline(always)]
pub fn update16(address: u32, clear: u32, set: u32) {
    write16(address, (read16(address) & !(clear as u16)) | set as u16);
}
#[inline(always)]
pub fn update32(address: u32, clear: u32, set: u32) {
    write32(address, (read32(address) & !clear) | set);
}
#[inline(always)]
pub fn write_physical_word(address: u32, value: u32) {
    Bus::write_physical_word(address, value);
}
