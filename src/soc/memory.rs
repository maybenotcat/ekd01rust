//! Linker-defined application memory ranges; hardware implementation is in HAL.
unsafe extern "C" {
    static __flash_image_end: u8;
    static __heap_start: u8;
    static __heap_end: u8;
}
type Hardware = mt2502_hal::memory::Memory<super::Environment>;
pub struct Memory;
impl Memory {
    pub fn heap_range() -> (usize, usize) {
        let start = core::ptr::addr_of!(__heap_start) as usize;
        (start, core::ptr::addr_of!(__heap_end) as usize - start)
    }
    pub fn probe_heap() -> bool {
        // The linker reserves this range; entry has not initialized the allocator.
        unsafe {
            Hardware::probe_heap(
                core::ptr::addr_of!(__heap_start) as u32,
                core::ptr::addr_of!(__heap_end) as u32,
            )
        }
    }
    pub fn initialize_cache() {
        Hardware::initialize_cache(core::ptr::addr_of!(__flash_image_end) as u32);
    }
}
