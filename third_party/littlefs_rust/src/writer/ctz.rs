use ::alloc::vec::Vec;

use crate::types::{Config, Error, Result};

use super::block::{erase_block, image_block_mut, program};

/// CTZ file representation for fresh-image writing.
///
/// CTZ blocks are indexed in logical order. Block 0 is pure data. Later blocks
/// start with skip pointers to earlier blocks, then store data. Pointer `k` in
/// block `i` points to block `i - 2^k`, which is exactly what `lfs_ctz_find`
/// follows when jumping backward from the head.
#[derive(Debug, Clone)]
pub(super) struct CtzFile {
    data: Vec<u8>,
    blocks: Vec<u32>,
}

impl CtzFile {
    pub(super) fn new(data: &[u8], blocks: Vec<u32>) -> Self {
        Self {
            data: data.to_vec(),
            blocks,
        }
    }

    pub(super) fn write_blocks(&self, image: &mut [u8], cfg: Config) -> Result<()> {
        let mut data_off = 0usize;
        for (index, block_id) in self.blocks.iter().copied().enumerate() {
            let block = image_block_mut(image, cfg, block_id)?;
            let data_start = ctz_data_start(index)?;

            if index > 0 {
                let skips = index.trailing_zeros() as usize + 1;
                for skip in 0..skips {
                    let target_index = index.checked_sub(1usize << skip).ok_or(Error::Corrupt)?;
                    let target = self.blocks[target_index];
                    program(block, skip * 4, &target.to_le_bytes())?;
                }
            }

            let capacity = cfg
                .block_size
                .checked_sub(data_start)
                .ok_or(Error::InvalidConfig)?;
            let remaining = self
                .data
                .len()
                .checked_sub(data_off)
                .ok_or(Error::Corrupt)?;
            let chunk_len = core::cmp::min(capacity, remaining);
            let chunk = self
                .data
                .get(data_off..data_off + chunk_len)
                .ok_or(Error::Corrupt)?;
            program(block, data_start, chunk)?;
            data_off += chunk_len;
        }

        if data_off != self.data.len() {
            return Err(Error::Corrupt);
        }
        Ok(())
    }

    pub(super) fn erase_blocks(&self, image: &mut [u8], cfg: Config) -> Result<()> {
        for block in &self.blocks {
            erase_block(image, cfg, *block)?;
        }
        Ok(())
    }

    pub(super) fn head(&self) -> Result<u32> {
        self.blocks.last().copied().ok_or(Error::Corrupt)
    }

    pub(super) fn len(&self) -> usize {
        self.data.len()
    }
}

fn ctz_data_start(index: usize) -> Result<usize> {
    if index == 0 {
        Ok(0)
    } else {
        let skips = index.trailing_zeros() as usize + 1;
        skips.checked_mul(4).ok_or(Error::NoSpace)
    }
}
