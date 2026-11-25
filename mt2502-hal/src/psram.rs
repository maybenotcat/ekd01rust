use super::{mmio::*, registers::*};

pub struct Psram<P: crate::environment::Trace>(core::marker::PhantomData<P>);
impl<P: crate::environment::Trace> Psram<P> {
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
}

pub fn probe_locations(locations: [u32; 3]) -> bool {
    // Preserve every location: the application has not initialized its RAM yet.
    let saved = locations.map(read32);
    for (index, address) in locations.iter().enumerate() {
        write32(*address, 0xa55a_1234 ^ index as u32);
    }
    let valid = locations
        .iter()
        .enumerate()
        .all(|(index, address)| read32(*address) == 0xa55a_1234 ^ index as u32);
    for (address, value) in locations.into_iter().zip(saved) {
        write32(address, value);
    }
    valid
}
