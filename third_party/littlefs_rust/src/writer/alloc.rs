use ::alloc::{vec, vec::Vec};

use crate::types::{Config, Error, Result};

/// Explicit allocation map for fresh in-memory images.
///
/// This still is not a full littlefs allocator: it does not scan an existing
/// filesystem, relocate metadata pairs, or free unreachable CTZ blocks. It does
/// keep a real used-block bitmap for images built in this process, which gives
/// future deallocation and wear-leveling work a concrete invariant to extend:
/// no block may be handed out twice.
#[derive(Debug, Clone)]
pub(super) struct FreshAllocator {
    cfg: Config,
    used: Vec<bool>,
}

impl FreshAllocator {
    pub(super) fn new(cfg: Config) -> Self {
        let mut used = vec![false; cfg.block_count];
        // Blocks 0 and 1 are the root metadata pair in every image this writer
        // formats. Marking them here makes later pair/data allocation obey the
        // same single source of truth as future imported-image allocators.
        if let Some(block) = used.get_mut(0) {
            *block = true;
        }
        if let Some(block) = used.get_mut(1) {
            *block = true;
        }
        Self { cfg, used }
    }

    pub(super) fn from_used(cfg: Config, used: Vec<bool>) -> Result<Self> {
        if used.len() != cfg.block_count {
            return Err(Error::InvalidConfig);
        }
        Ok(Self { cfg, used })
    }

    pub(super) fn into_used(self) -> Vec<bool> {
        self.used
    }

    pub(super) fn alloc_pair(&mut self) -> Result<[u32; 2]> {
        let blocks = self.find_free_blocks(2)?;
        if blocks.len() != 2 {
            return Err(Error::NoSpace);
        }
        self.reserve_block(blocks[0])?;
        self.reserve_block(blocks[1])?;
        // Match the pair order that C publishes after the initial metadata
        // compaction succeeds. `lfs_dir_alloc` fills an internal pair
        // backwards, writes pair[1], then swaps; fresh Rust images start from
        // the already-committed representation stored in parent metadata.
        Ok([blocks[0], blocks[1]])
    }

    pub(super) fn alloc_ctz_blocks(&mut self, size: usize) -> Result<Vec<u32>> {
        if size == 0 {
            return Err(Error::Unsupported);
        }
        let count = ctz_block_count(size, self.cfg.block_size)?;
        let blocks = self.find_free_blocks(count)?;
        for block in &blocks {
            self.reserve_block(*block)?;
        }
        Ok(blocks)
    }

    fn find_free_blocks(&self, count: usize) -> Result<Vec<u32>> {
        let mut blocks = Vec::new();
        for (block, used) in self.used.iter().enumerate() {
            if !*used {
                blocks.push(u32::try_from(block).map_err(|_| Error::NoSpace)?);
                if blocks.len() == count {
                    return Ok(blocks);
                }
            }
        }
        Err(Error::NoSpace)
    }

    fn reserve_block(&mut self, block: u32) -> Result<()> {
        let slot = self
            .used
            .get_mut(block as usize)
            .ok_or(Error::OutOfBounds)?;
        if *slot {
            return Err(Error::Corrupt);
        }
        *slot = true;
        Ok(())
    }
}

fn ctz_block_count(size: usize, block_size: usize) -> Result<usize> {
    let mut remaining = size;
    let mut index = 0usize;
    while remaining > 0 {
        let start = ctz_data_start(index)?;
        let capacity = block_size.checked_sub(start).ok_or(Error::InvalidConfig)?;
        if capacity == 0 {
            return Err(Error::InvalidConfig);
        }
        remaining = remaining.saturating_sub(capacity);
        index = index.checked_add(1).ok_or(Error::NoSpace)?;
    }
    Ok(index)
}

fn ctz_data_start(index: usize) -> Result<usize> {
    if index == 0 {
        Ok(0)
    } else {
        let skips = index.trailing_zeros() as usize + 1;
        skips.checked_mul(4).ok_or(Error::NoSpace)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocator_marks_root_pair_and_rejects_exhaustion_without_reuse() {
        let mut allocator = FreshAllocator::new(Config {
            block_size: 512,
            block_count: 6,
        });

        assert_eq!(allocator.alloc_pair().expect("first child pair"), [2, 3]);
        assert_eq!(
            allocator
                .alloc_ctz_blocks(32)
                .expect("single small CTZ block"),
            vec![4]
        );
        assert_eq!(
            allocator
                .alloc_pair()
                .expect_err("only one block remains, so a pair cannot fit"),
            Error::NoSpace
        );
        assert_eq!(
            allocator
                .alloc_ctz_blocks(32)
                .expect("the remaining block is still usable after failed pair allocation"),
            vec![5]
        );
    }
}
