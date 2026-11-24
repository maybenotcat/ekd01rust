// N22 COMBO_MEM_SFI entry 2, with its device command-list types decoded.
pub const JEDEC: [u8; 3] = [0xc8, 0x60, 0x17];
pub const HW_CONF_130M: [u32; 7] = [
    0x0001_0000,
    0x0c0b_5771,
    0x52f8_0010,
    1,
    0,
    0x1300_0013,
    0x0004_0004,
];
pub const DEVICE_COMMANDS: &[(bool, &[u8])] = &[
    (false, &[0x50]),
    (false, &[0x01, 0x02, 0x02]),
    (false, &[0x38]),
    (true, &[0xc0, 0x22]),
];
