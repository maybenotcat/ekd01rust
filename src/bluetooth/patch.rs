//! Original N22 controller patch: the file's 30-byte metadata header is not HCI payload.
pub const HEADER_BYTES: usize = 30;
pub const PAYLOAD: &[u8] = include_bytes!("../../firmware/original/bt-patch.bin")
    .split_at(HEADER_BYTES)
    .1;
