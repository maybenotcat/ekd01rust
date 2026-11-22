use alloc::vec::Vec;

use crate::types::{Error, Result};

// These constants mirror the lfs2.1 on-disk tag type values. Keeping them in
// one module avoids scattering magic numbers through the parser and makes it
// obvious which code is coupled to the disk format.
pub(crate) const LFS_TYPE_REG: u16 = 0x001;
pub(crate) const LFS_TYPE_DIR: u16 = 0x002;
pub(crate) const LFS_TYPE_SUPERBLOCK: u16 = 0x0ff;
pub(crate) const LFS_TYPE_DIRSTRUCT: u16 = 0x200;
pub(crate) const LFS_TYPE_INLINESTRUCT: u16 = 0x201;
pub(crate) const LFS_TYPE_CTZSTRUCT: u16 = 0x202;
pub(crate) const LFS_TYPE_USERATTR: u16 = 0x300;
pub(crate) const LFS_TYPE_CREATE: u16 = 0x401;
pub(crate) const LFS_TYPE_DELETE: u16 = 0x4ff;
pub(crate) const LFS_TYPE_SOFTTAIL: u16 = 0x600;
pub(crate) const LFS_TYPE_HARDTAIL: u16 = 0x601;
pub(crate) const LFS_TYPE_MOVESTATE: u16 = 0x7ff;
pub(crate) const LFS_TYPE_CCRC_MASK: u16 = 0x780;
pub(crate) const LFS_TYPE_CCRC_VALUE: u16 = 0x500;
pub(crate) const LFS_TYPE_FCRC: u16 = 0x5ff;
pub(crate) const LFS_NULL: u32 = 0xffff_ffff;

/// A committed metadata entry after the XOR tag has been decoded.
///
/// The `data` slice intentionally excludes the 32-bit tag word. For CRC tags,
/// `data` still includes the little-endian CRC payload and any padding bytes.
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) tag: Tag,
    pub(crate) data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Tag(pub(crate) u32);

impl Tag {
    /// Builds the in-memory tag value used by littlefs.
    ///
    /// Tags are stored on disk as big-endian XOR deltas between neighboring
    /// tags, but once decoded the C implementation treats them as this packed
    /// integer: valid bit, type3, id, and size.
    pub(crate) fn new(type3: u16, id: u16, size: u16) -> Self {
        Self(((type3 as u32) << 20) | ((id as u32) << 10) | size as u32)
    }

    /// The high bit is the validity bit. After decoding, valid committed tags
    /// have this bit cleared.
    pub(crate) fn is_valid(self) -> bool {
        self.0 & 0x8000_0000 == 0
    }

    /// littlefs encodes deletion by using the all-ones size field.
    pub(crate) fn is_delete(self) -> bool {
        self.size() == 0x3ff
    }

    pub(crate) fn type3(self) -> u16 {
        ((self.0 & 0x7ff0_0000) >> 20) as u16
    }

    pub(crate) fn id(self) -> u16 {
        ((self.0 & 0x000f_fc00) >> 10) as u16
    }

    pub(crate) fn size(self) -> u16 {
        (self.0 & 0x0000_03ff) as u16
    }

    pub(crate) fn dsize(self) -> Result<usize> {
        // Delete tags carry no payload even though their size bits are 0x3ff.
        if self.is_delete() {
            Ok(4)
        } else {
            Ok(4 + self.size() as usize)
        }
    }

    pub(crate) fn is_ccrc(self) -> bool {
        (self.type3() & LFS_TYPE_CCRC_MASK) == LFS_TYPE_CCRC_VALUE
    }

    /// Returns the lookup mask that littlefs uses when asking whether a later
    /// tag supersedes an earlier one. Unique tags match at full type3 precision,
    /// while grouped tags such as names/userattrs use broader masks.
    pub(crate) fn lookup_mask(self) -> Self {
        let is_unique = self.0 & 0x1000_0000 == 0;
        let is_attr = self.0 & 0x4000_0000 == 0;
        Self(
            ((if is_unique { 0x7ff } else { 0x700 }) << 20)
                | ((if is_attr { 0x3ff } else { 0 }) << 10),
        )
    }

    pub(crate) fn matches(self, mask: Tag, want: Tag) -> bool {
        (self.0 & mask.0) == (want.0 & mask.0)
    }
}

pub(crate) fn le32(data: &[u8]) -> Result<u32> {
    let bytes: [u8; 4] = data.try_into().map_err(|_| Error::Corrupt)?;
    Ok(u32::from_le_bytes(bytes))
}

/// Tags in commits are the one big-endian exception in littlefs. Most other
/// multi-byte on-disk values are little-endian.
pub(crate) fn be32(data: &[u8]) -> Result<u32> {
    let bytes: [u8; 4] = data.try_into().map_err(|_| Error::Corrupt)?;
    Ok(u32::from_be_bytes(bytes))
}

/// Sequence comparison for metadata block revisions. Revisions are 32-bit and
/// may wrap, so normal integer ordering is not safe.
pub(crate) fn seq_after(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) & 0x8000_0000) == 0
}

pub(crate) fn ctz(x: u32) -> u32 {
    x.trailing_zeros()
}

pub(crate) fn popc(x: u32) -> u32 {
    x.count_ones()
}

pub(crate) fn npw2(x: u32) -> u32 {
    if x <= 1 {
        0
    } else {
        u32::BITS - (x - 1).leading_zeros()
    }
}

/// Small lookup-table CRC used by littlefs. This is deliberately equivalent to
/// `lfs_crc` in `littlefs_c/lfs_util.c`.
pub(crate) fn crc32(mut crc: u32, data: &[u8]) -> u32 {
    const RTABLE: [u32; 16] = [
        0x00000000, 0x1db71064, 0x3b6e20c8, 0x26d930ac, 0x76dc4190, 0x6b6b51f4, 0x4db26158,
        0x5005713c, 0xedb88320, 0xf00f9344, 0xd6d6a3e8, 0xcb61b38c, 0x9b64c2b0, 0x86d3d2d4,
        0xa00ae278, 0xbdbdf21c,
    ];

    for byte in data {
        crc = (crc >> 4) ^ RTABLE[((crc ^ (*byte as u32)) & 0xf) as usize];
        crc = (crc >> 4) ^ RTABLE[((crc ^ ((*byte as u32) >> 4)) & 0xf) as usize];
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_known_littlefs_vector() {
        assert_eq!(crc32(0xffff_ffff, b"littlefs"), 0x5a11_385b);
    }
}
