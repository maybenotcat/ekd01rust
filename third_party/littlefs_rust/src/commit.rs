use alloc::vec::Vec;

use crate::{
    format::{LFS_TYPE_CCRC_VALUE, Tag, crc32},
    types::{Error, Result},
};

/// One semantic metadata entry waiting to be encoded into an on-disk commit.
///
/// The tag is stored in littlefs's in-memory packed form. `MetadataCommitWriter`
/// handles the on-disk XOR delta and big-endian encoding, so higher-level
/// writers can stay focused on which tags should exist rather than how littlefs
/// serializes them.
#[derive(Debug, Clone)]
pub(crate) struct CommitEntry {
    tag: Tag,
    data: Vec<u8>,
}

impl CommitEntry {
    pub(crate) fn new(tag: Tag, data: &[u8]) -> Self {
        Self {
            tag,
            data: data.to_vec(),
        }
    }

    pub(crate) fn tag(&self) -> Tag {
        self.tag
    }

    pub(crate) fn data(&self) -> &[u8] {
        &self.data
    }
}

/// Resume point after a completed metadata commit.
///
/// littlefs appends multiple commits inside the same metadata block. The next
/// commit starts at the byte after the previous CCRC's padding, and its first
/// tag is XORed against the previous decoded CCRC tag. Keeping both values
/// together prevents higher-level code from guessing at this fragile state.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CommitState {
    pub(crate) off: usize,
    pub(crate) ptag: u32,
}

/// Low-level writer for one littlefs metadata block commit.
///
/// This type owns no allocation policy and knows nothing about directories or
/// files. It only mirrors the mechanics in C's `lfs_dir_commitattr` and
/// `lfs_dir_commitcrc`: revision word, XORed big-endian tags, running CRC,
/// erased padding, and NOR-style programming into an already-erased block.
pub(crate) struct MetadataCommitWriter<'a> {
    block: &'a mut [u8],
    prog_size: usize,
    off: usize,
    ptag: u32,
    crc: u32,
}

impl<'a> MetadataCommitWriter<'a> {
    pub(crate) fn new(block: &'a mut [u8], prog_size: usize) -> Result<Self> {
        if prog_size == 0 || !prog_size.is_power_of_two() {
            return Err(Error::InvalidConfig);
        }
        Ok(Self {
            block,
            prog_size,
            off: 0,
            ptag: 0xffff_ffff,
            crc: 0xffff_ffff,
        })
    }

    pub(crate) fn append(
        block: &'a mut [u8],
        prog_size: usize,
        state: CommitState,
    ) -> Result<Self> {
        if prog_size == 0 || !prog_size.is_power_of_two() {
            return Err(Error::InvalidConfig);
        }
        if state.off > block.len() {
            return Err(Error::NoSpace);
        }
        Ok(Self {
            block,
            prog_size,
            off: state.off,
            ptag: state.ptag,
            // C resets the commit CRC after every valid CCRC. Appended commits
            // therefore start from the seed, not from the previous commit's
            // final CRC value.
            crc: 0xffff_ffff,
        })
    }

    /// Writes the little-endian metadata block revision.
    ///
    /// The revision participates in the first commit CRC just like in C. A
    /// caller that wants to append to an existing metadata block will need a
    /// different constructor later; the current writer is deliberately scoped
    /// to compacting a fresh block from offset zero.
    pub(crate) fn write_revision(&mut self, rev: u32) -> Result<()> {
        self.write_raw(&rev.to_le_bytes())
    }

    pub(crate) fn write_entries(&mut self, entries: &[CommitEntry]) -> Result<()> {
        for entry in entries {
            self.write_entry(entry)?;
        }
        Ok(())
    }

    /// Terminates the commit with a CCRC tag and returns the next unwritten
    /// offset and XOR base needed by a later append.
    pub(crate) fn finish(&mut self) -> Result<CommitState> {
        self.write_commit_crc()?;
        Ok(CommitState {
            off: self.off,
            ptag: self.ptag,
        })
    }

    fn write_entry(&mut self, entry: &CommitEntry) -> Result<()> {
        let expected_size = entry.tag.dsize()?.checked_sub(4).ok_or(Error::Corrupt)?;
        if expected_size != entry.data.len() {
            return Err(Error::Corrupt);
        }

        // On disk, metadata tags are big-endian XOR deltas from the previous
        // decoded tag. Payload bytes follow as-is and are included in the
        // running CRC for non-CRC tags.
        let disk_tag = ((entry.tag.0 & 0x7fff_ffff) ^ self.ptag).to_be_bytes();
        self.write_raw(&disk_tag)?;
        self.write_raw(&entry.data)?;
        self.ptag = entry.tag.0 & 0x7fff_ffff;
        Ok(())
    }

    fn write_commit_crc(&mut self) -> Result<()> {
        // CCRC payload size is chosen so the next offset lands on a program
        // boundary. The first four payload bytes store the little-endian CRC;
        // any remaining bytes are left erased and are not included in the CRC.
        let payload_size = self.ccrc_payload_size()?;
        let tag = Tag::new(LFS_TYPE_CCRC_VALUE, 0x3ff, checked_u10(payload_size)?);
        let disk_tag = ((tag.0 & 0x7fff_ffff) ^ self.ptag).to_be_bytes();
        self.write_raw(&disk_tag)?;

        let crc = self.crc.to_le_bytes();
        self.program(&crc)?;
        self.off += crc.len();

        let padding = payload_size.checked_sub(4).ok_or(Error::Corrupt)?;
        self.off += padding;
        self.ptag = tag.0 & 0x7fff_ffff;
        self.crc = 0xffff_ffff;
        Ok(())
    }

    fn ccrc_payload_size(&self) -> Result<usize> {
        let unaligned_end = self.off.checked_add(8).ok_or(Error::NoSpace)?;
        let aligned_end = align_up(unaligned_end, self.prog_size)?;
        let payload_size = aligned_end
            .checked_sub(self.off + 4)
            .ok_or(Error::Corrupt)?;
        if payload_size < 4 {
            return Err(Error::Corrupt);
        }
        Ok(payload_size)
    }

    fn write_raw(&mut self, data: &[u8]) -> Result<()> {
        self.program(data)?;
        self.crc = crc32(self.crc, data);
        self.off += data.len();
        Ok(())
    }

    fn program(&mut self, data: &[u8]) -> Result<()> {
        let end = self.off.checked_add(data.len()).ok_or(Error::NoSpace)?;
        if end > self.block.len() {
            return Err(Error::NoSpace);
        }

        // Metadata blocks are expected to be erased before this compacting
        // writer runs. Programming with `&=` preserves NOR flash semantics and
        // also makes any accidental program-without-erase behavior visible in
        // tests instead of silently overwriting bits from 0 back to 1.
        for (dst, src) in self.block[self.off..end].iter_mut().zip(data) {
            *dst &= *src;
        }
        Ok(())
    }
}

pub(crate) fn checked_u10(value: usize) -> Result<u16> {
    if value > 0x3fe {
        return Err(Error::Unsupported);
    }
    Ok(value as u16)
}

fn align_up(value: usize, align: usize) -> Result<usize> {
    if align == 0 || !align.is_power_of_two() {
        return Err(Error::InvalidConfig);
    }
    value
        .checked_add(align - 1)
        .map(|value| value & !(align - 1))
        .ok_or(Error::NoSpace)
}
