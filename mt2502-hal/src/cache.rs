//! XIP cache programming. Caller supplies executable range and SRAM execution.
use crate::{mmio::*, registers::*};
/// # Safety
/// Execute from SRAM after installing this routine; no concurrent CPU/DMA user
/// may access the cache while mappings change. The supplied range must describe
/// executable NOR. This function does not discover a binary's linker layout.
#[unsafe(link_section = ".ramfunc")]
#[inline(never)]
pub unsafe fn configure_xip(start: u32, end: u32) {
    let control = read16(CACHE_CON) as u32;
    write16(CACHE_CON, (control & !EKD_CACHE_DISABLE_MASK) as u16);
    write32(CACHE_OP, EKD_CACHE_INVALIDATE_ALL);
    for offset in (8..=0x24).step_by(4) {
        write32(L1_CACHE_base + offset, 0);
    }
    write32(CACHE_REGION_EN, 0);
    for channel in 0..16 {
        write32(MPU_PROT(channel), 0);
        write32(MPU_CACHE(channel), 0);
    }
    write32(MPU_PROT(0), start);
    write32(MPU_CACHE(0), end);
    write32(CACHE_REGION_EN, 1);
    write16(
        CACHE_CON,
        ((control & !EKD_CACHE_RECONFIGURE_MASK) | EKD_CACHE_32K_4WAY_CONTROL) as u16,
    );
    let _ = read16(CACHE_CON);
}

pub fn control() -> u16 {
    read16(CACHE_CON)
}
pub fn invalidate_and_disable() {
    update16(CACHE_CON, EKD_CACHE_DISABLE_MASK, 0);
    write32(CACHE_OP, EKD_CACHE_INVALIDATE_ALL);
    write32(CACHE_REGION_EN, 0);
}
pub fn disable_for_jump() {
    update16(CACHE_CON, 3, 0);
    write32(CACHE_REGION_EN, 0);
}
