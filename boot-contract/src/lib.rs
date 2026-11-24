//! Shared physical NOR layout and application header validation.
#![no_std]

pub const NOR_BASE: u32 = 0x1000_0000;
pub const NOR_BYTES: u32 = 0x0080_0000;
// Leave the original internal SRAM stack at 70009000..7000a000 untouched.
// The former 8KiB cap was a conservative assumption, not a GFH/ROM field.
// The observed ROM rejection was traced to BRLYT attributes, not image size.
pub const BOOT_LOAD_ADDRESS: u32 = 0x7000_6000;
pub const BOOT_ROM_END: u32 = 0x7000_9000;
pub const BOOT_HASH_BYTES: u32 = 32;
pub const ERASE_BYTES: u32 = 4096;
pub const WRITE_BYTES: u32 = 4;
// Outside application RAM, low vectors and the original BootROM work area.
pub const HANDOFF_ADDRESS: u32 = 0x0003_f000;
pub const HANDOFF_MAGIC: u32 = 0x454b_4442;
pub const HANDOFF_VERSION: u32 = 1;
/// Embassy Boot 0.7.0 SWAP magic, repeated for one STATE write word.
pub const PENDING_MARKER: [u8; WRITE_BYTES as usize] = [0xf0; WRITE_BYTES as usize];
pub const APPLICATION_HEADER_BYTES: u32 = 0x520;
pub const APPLICATION_ENTRY: u32 = NOR_BASE + ACTIVE.start + APPLICATION_HEADER_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Partition {
    pub start: u32,
    pub end: u32,
}

impl Partition {
    pub const fn len(self) -> u32 {
        self.end - self.start
    }

    /// Convert a partition-relative operation to a physical NOR address.
    pub const fn address(self, offset: u32, length: usize) -> Option<u32> {
        if offset > self.len() || length > (self.len() - offset) as usize {
            None
        } else {
            Some(self.start + offset)
        }
    }
}

pub const ACTIVE: Partition = Partition {
    start: 0x050000,
    end: 0x250000,
};
pub const DFU: Partition = Partition {
    start: 0x250000,
    end: 0x451000,
};
pub const STATE: Partition = Partition {
    start: 0x451000,
    end: 0x455000,
};
pub const SETTINGS: Partition = Partition {
    start: 0x500000,
    end: NOR_BYTES,
};

// Embassy Boot stores two magic words and four progress words per active page.
const _: () = assert!(DFU.len() == ACTIVE.len() + ERASE_BYTES);
const _: () = assert!(STATE.len() >= (2 + 4 * (ACTIVE.len() / ERASE_BYTES)) * WRITE_BYTES);
const _: () = assert!(ACTIVE.end == DFU.start && DFU.end == STATE.start);
const _: () = assert!(STATE.end <= SETTINGS.start);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageError {
    Truncated,
    Header,
    Address,
    Size,
    Vector,
}

/// Validate the unsigned GFH application envelope before executing its vectors.
/// This checks structure and bounds; it is not cryptographic authentication.
pub fn validate_application(header_and_vectors: &[u8]) -> Result<u32, ImageError> {
    let word = |offset: usize| -> Result<u32, ImageError> {
        let bytes = header_and_vectors
            .get(offset..offset + 4)
            .ok_or(ImageError::Truncated)?;
        Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
    };
    if header_and_vectors.get(..4) != Some(b"MMM\x01")
        || header_and_vectors.get(8..17) != Some(b"FILE_INFO")
        || header_and_vectors.get(24..28) != Some(&[0x00, 0x01, 0x07, 0x00])
    {
        return Err(ImageError::Header);
    }
    if word(28)? != NOR_BASE + ACTIVE.start
        || word(40)? != APPLICATION_HEADER_BYTES
        || word(48)? != APPLICATION_HEADER_BYTES
    {
        return Err(ImageError::Address);
    }
    let size = word(32)?;
    if size < APPLICATION_HEADER_BYTES + 64 || size > ACTIVE.len() || word(44)? != 0 {
        return Err(ImageError::Size);
    }
    for vector in 0..8 {
        let instruction = APPLICATION_HEADER_BYTES as usize + vector * 4;
        let target = word(instruction + 32)?;
        if word(instruction)? != 0xe59f_f018
            || target & 3 != 0
            || target < APPLICATION_ENTRY + 64
            || target >= NOR_BASE + ACTIVE.start + size
        {
            return Err(ImageError::Vector);
        }
    }
    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partition_operations_cannot_escape_or_wrap() {
        assert_eq!(ACTIVE.address(0, ACTIVE.len() as usize), Some(ACTIVE.start));
        assert_eq!(ACTIVE.address(ACTIVE.len(), 0), Some(ACTIVE.end));
        assert_eq!(ACTIVE.address(ACTIVE.len(), 1), None);
        assert_eq!(ACTIVE.address(u32::MAX, 4), None);
        assert_eq!(ACTIVE.address(1, usize::MAX), None);
    }

    #[test]
    fn application_header_rejects_other_addresses_and_oversized_images() {
        let mut bytes = [0u8; APPLICATION_HEADER_BYTES as usize + 68];
        bytes[..4].copy_from_slice(b"MMM\x01");
        bytes[8..17].copy_from_slice(b"FILE_INFO");
        bytes[24..28].copy_from_slice(&[0, 1, 7, 0]);
        for (offset, value) in [
            (28, NOR_BASE + ACTIVE.start),
            (32, bytes.len() as u32),
            (40, APPLICATION_HEADER_BYTES),
            (48, APPLICATION_HEADER_BYTES),
            (APPLICATION_HEADER_BYTES as usize, 0xe59f_f018),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        for vector in 0..8 {
            let offset = APPLICATION_HEADER_BYTES as usize + vector * 4;
            bytes[offset..offset + 4].copy_from_slice(&0xe59f_f018u32.to_le_bytes());
            bytes[offset + 32..offset + 36]
                .copy_from_slice(&(APPLICATION_ENTRY + 64).to_le_bytes());
        }
        assert_eq!(validate_application(&bytes), Ok(bytes.len() as u32));
        let target = APPLICATION_HEADER_BYTES as usize + 32;
        bytes[target..target + 4].copy_from_slice(&0x7000_0000u32.to_le_bytes());
        assert_eq!(validate_application(&bytes), Err(ImageError::Vector));
        bytes[target..target + 4].copy_from_slice(&(APPLICATION_ENTRY + 64).to_le_bytes());
        bytes[32..36].copy_from_slice(&(ACTIVE.len() + 4).to_le_bytes());
        assert_eq!(validate_application(&bytes), Err(ImageError::Size));
        bytes[28..32].copy_from_slice(&0x1000_0000u32.to_le_bytes());
        assert_eq!(validate_application(&bytes), Err(ImageError::Address));
    }
}
