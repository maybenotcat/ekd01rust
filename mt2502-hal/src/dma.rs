//! CPU-to-DMA address conversion for this uncached physical-memory platform.
pub fn physical_address(pointer: *const u16) -> u32 {
    #[cfg(not(feature = "register-tests"))]
    {
        pointer as u32
    }
    #[cfg(feature = "register-tests")]
    {
        crate::testing::dma_address(pointer)
    }
}
