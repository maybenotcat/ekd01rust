pub fn crc32(bytes: &[u8]) -> u32 {
    let mut value = !0u32;
    for byte in bytes {
        value ^= *byte as u32;
        for _ in 0..8 {
            value = (value >> 1) ^ 0u32.wrapping_sub(value & 1) & 0xedb88320;
        }
    }
    !value
}
