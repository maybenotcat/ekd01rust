use super::{mmio::*, registers::*, watchdog::Watchdog};

pub struct Memory<P: crate::environment::Trace>(core::marker::PhantomData<P>);
impl<P: crate::environment::Trace> Memory<P> {
    /// Destructively probe the linker-reserved, not-yet-allocated heap.
    ///
    /// # Safety
    /// The complete range must be initialized uncached PSRAM, reserved for
    /// this probe and contain no live Rust objects or DMA buffers.
    pub unsafe fn probe_heap(begin: u32, end: u32) -> bool {
        P::hex("[PSRAM] EMI_GENA", read32(EMI_GENA));
        for bit in 0..32 {
            let value = 1 << bit;
            write32(begin, value);
            if read32(begin) != value {
                return false;
            }
            write32(begin, !value);
            if read32(begin) != !value {
                return false;
            }
        }
        write32(begin, 0x51a7c0de);
        let mut offset = 4;
        while begin + offset < end {
            write32(begin + offset, offset ^ 0xa55a1234);
            if read32(begin) != 0x51a7c0de {
                return false;
            }
            offset <<= 1;
        }
        offset = 4;
        while begin + offset < end {
            if read32(begin + offset) != offset ^ 0xa55a1234 {
                return false;
            }
            offset <<= 1;
        }
        P::log("[PSRAM] HEAP_ADDRESS_DATA_PROBE_OK\n");
        true
    }
    pub fn initialize_cache(image_end: u32) {
        let end = (image_end + 4095) & !4095;
        critical_section::with(|_| configure_cache(end));
        Watchdog::feed();
        P::hex("[CACHE] CON", read16(CACHE_CON) as u32);
        P::log("[CACHE] XIP_32K_4WAY; uncached physical PSRAM\n");
    }
}

#[unsafe(link_section = ".ramfunc")]
#[inline(never)]
fn configure_cache(end: u32) {
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
    write32(MPU_PROT(0), EKD_CACHE_XIP_START);
    write32(MPU_CACHE(0), end);
    write32(CACHE_REGION_EN, 1);
    write16(
        CACHE_CON,
        ((control & !EKD_CACHE_RECONFIGURE_MASK) | EKD_CACHE_32K_4WAY_CONTROL) as u16,
    );
    let _ = read16(CACHE_CON);
}
