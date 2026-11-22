use alloc::vec::Vec;

use crate::types::{Config, Error, Result};

/// Minimal mounted block allocator.
///
/// This is not littlefs's final lookahead allocator yet. It deliberately keeps
/// a simple in-memory used-block bitmap plus a next-fit cursor so mounted
/// native writes can allocate without rescanning every reachable metadata/CTZ
/// block on each file growth. Native writes clone/commit the allocator state
/// only after the corresponding metadata transaction succeeds, and operations
/// that free visible storage rebuild it from the freshly remounted filesystem.
#[derive(Debug, Clone)]
pub(crate) struct BlockAllocator {
    cfg: Config,
    used: Vec<bool>,
    bad: Option<Vec<bool>>,
    lookahead: Vec<bool>,
    lookahead_start: usize,
    next: usize,
}

impl BlockAllocator {
    #[cfg(test)]
    pub(crate) fn from_used(cfg: Config, used: Vec<bool>) -> Result<Self> {
        Self::from_used_with_start(cfg, used, 0)
    }

    /// Builds an allocator whose first lookahead scan begins at `start`.
    ///
    /// On mount, C littlefs derives this start point from a CRC seed over
    /// committed metadata blocks. Accepting an explicit start keeps the simple
    /// bitmap allocator testable while letting mounted filesystems avoid the
    /// old Rust-native behavior of always reusing the lowest free block after a
    /// remount.
    pub(crate) fn from_used_with_start(cfg: Config, used: Vec<bool>, start: usize) -> Result<Self> {
        Self::from_used_with_bad_blocks(cfg, used, start, &[])
    }

    /// Rebuilds the allocator from a freshly visible used-block graph while
    /// preserving blocks that failed with `Error::Corrupt` in the current
    /// mounted session.
    ///
    /// littlefs does not expose a portable persistent bad-block table in the
    /// on-disk format, but it also must not immediately reuse an allocation
    /// candidate that just reported a bad-block style failure. Keeping the
    /// reservation in the live allocator is a pragmatic middle ground: failed
    /// unreachable blocks stay unavailable for the rest of the mount, while a
    /// future remount starts from the disk-visible graph just like C.
    pub(crate) fn from_used_with_bad_blocks(
        cfg: Config,
        mut used: Vec<bool>,
        start: usize,
        bad: &[bool],
    ) -> Result<Self> {
        if used.len() != cfg.block_count {
            return Err(Error::InvalidConfig);
        }
        if cfg.block_count == 0 {
            return Err(Error::InvalidConfig);
        }
        let mut bad_blocks = None;
        if !bad.is_empty() {
            if bad.len() != cfg.block_count {
                return Err(Error::InvalidConfig);
            }
            let mut blocks = alloc::vec![false; cfg.block_count];
            blocks.copy_from_slice(bad);
            for (slot, is_bad) in used.iter_mut().zip(&blocks) {
                if *is_bad {
                    *slot = true;
                }
            }
            bad_blocks = Some(blocks);
        }
        let start = start % cfg.block_count;
        let mut allocator = Self {
            cfg,
            used,
            bad: bad_blocks,
            lookahead: alloc::vec![false; core::cmp::min(cfg.block_count, 64)],
            lookahead_start: start,
            next: start,
        };
        allocator.reload_lookahead(start);
        Ok(allocator)
    }

    pub(crate) fn alloc_block(&mut self) -> Result<u32> {
        let blocks = self.alloc_blocks(1)?;
        Ok(blocks[0])
    }

    pub(crate) fn alloc_pair(&mut self) -> Result<[u32; 2]> {
        let blocks = self.alloc_blocks(2)?;
        // C littlefs allocates a fresh `lfs_mdir_t` backwards internally, then
        // writes the first committed directory block to pair[1] and swaps the
        // pair after the commit sticks. This allocator returns the published
        // post-commit order that callers store in DIRSTRUCT/SOFTTAIL records.
        Ok([blocks[0], blocks[1]])
    }

    pub(crate) fn alloc_ctz_blocks(&mut self, size: usize) -> Result<Vec<u32>> {
        if size == 0 {
            return Err(Error::Unsupported);
        }
        let needed = ctz_block_count(size, self.cfg.block_size)?;
        self.alloc_blocks(needed)
    }

    pub(crate) fn reserve_bad_block(&mut self, block: u32) -> Result<()> {
        let block = block as usize;
        if block >= self.cfg.block_count {
            return Err(Error::OutOfBounds);
        }
        let bad = self
            .bad
            .get_or_insert_with(|| alloc::vec![false; self.cfg.block_count]);
        bad[block] = true;
        self.used[block] = true;
        if self.lookahead_slot(block).is_some() {
            self.set_lookahead_used(block)?;
        }
        Ok(())
    }

    pub(crate) fn bad_blocks(&self) -> &[bool] {
        self.bad.as_deref().unwrap_or(&[])
    }

    fn alloc_blocks(&mut self, needed: usize) -> Result<Vec<u32>> {
        if needed == 0 {
            return Ok(Vec::new());
        }
        if needed > self.cfg.block_count {
            return Err(Error::NoSpace);
        }

        let mut blocks = Vec::with_capacity(needed);
        let mut scanned = 0usize;
        let mut cursor = self.next;
        while scanned < self.cfg.block_count {
            if self.lookahead_slot(cursor).is_none() {
                self.reload_lookahead(cursor);
            }

            if !self.lookahead_used(cursor)? {
                self.used[cursor] = true;
                self.set_lookahead_used(cursor)?;
                blocks.push(u32::try_from(cursor).map_err(|_| Error::NoSpace)?);
                if blocks.len() == needed {
                    self.next = (cursor + 1) % self.cfg.block_count;
                    return Ok(blocks);
                }
            }
            cursor = (cursor + 1) % self.cfg.block_count;
            scanned += 1;
        }

        // Roll back any reservations from this failed allocation attempt. This
        // keeps metadata-pair allocation from consuming one leftover block when
        // a two-block pair cannot be satisfied.
        for block in blocks {
            let block = block as usize;
            self.used[block] = false;
            if self.lookahead_slot(block).is_some() {
                self.set_lookahead_free(block)?;
            }
        }
        Err(Error::NoSpace)
    }

    fn reload_lookahead(&mut self, start: usize) {
        self.lookahead_start = start;
        for (i, slot) in self.lookahead.iter_mut().enumerate() {
            let block = (start + i) % self.cfg.block_count;
            *slot = self.used[block];
        }
    }

    fn lookahead_slot(&self, block: usize) -> Option<usize> {
        let size = self.lookahead.len();
        if size == 0 {
            return None;
        }
        let end = self.lookahead_start + size;
        if self.lookahead_start <= block && block < end {
            Some(block - self.lookahead_start)
        } else if end > self.cfg.block_count && block < end % self.cfg.block_count {
            Some(self.cfg.block_count - self.lookahead_start + block)
        } else {
            None
        }
    }

    fn lookahead_used(&self, block: usize) -> Result<bool> {
        let slot = self.lookahead_slot(block).ok_or(Error::Corrupt)?;
        Ok(self.lookahead[slot])
    }

    fn set_lookahead_used(&mut self, block: usize) -> Result<()> {
        let slot = self.lookahead_slot(block).ok_or(Error::Corrupt)?;
        self.lookahead[slot] = true;
        Ok(())
    }

    fn set_lookahead_free(&mut self, block: usize) -> Result<()> {
        let slot = self.lookahead_slot(block).ok_or(Error::Corrupt)?;
        self.lookahead[slot] = false;
        Ok(())
    }

    #[cfg(test)]
    fn lookahead_window_for_test(&self) -> (usize, Vec<bool>) {
        (self.lookahead_start, self.lookahead.clone())
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
    fn pair_allocation_rolls_back_when_two_blocks_do_not_fit() {
        let cfg = Config {
            block_size: 512,
            block_count: 5,
        };
        let mut used = vec![true, true, true, true, false];
        let mut allocator = BlockAllocator::from_used(cfg, used.clone()).expect("allocator");
        assert_eq!(
            allocator
                .alloc_pair()
                .expect_err("one free block cannot satisfy a metadata pair"),
            Error::NoSpace
        );

        used[4] = false;
        assert_eq!(
            allocator
                .alloc_ctz_blocks(32)
                .expect("failed pair allocation must not consume the leftover block"),
            vec![4]
        );
    }

    #[test]
    fn ctz_allocation_uses_next_fit_cursor() {
        let cfg = Config {
            block_size: 512,
            block_count: 8,
        };
        let used = vec![true, true, false, false, false, false, false, false];
        let mut allocator = BlockAllocator::from_used(cfg, used).expect("allocator");
        assert_eq!(allocator.alloc_ctz_blocks(32).expect("first"), vec![2]);
        assert_eq!(allocator.alloc_ctz_blocks(32).expect("second"), vec![3]);
    }

    #[test]
    fn allocator_refreshes_lookahead_window_while_scanning() {
        let cfg = Config {
            block_size: 512,
            block_count: 96,
        };
        let mut used = vec![true; 96];
        used[70] = false;
        let mut allocator = BlockAllocator::from_used(cfg, used).expect("allocator");

        assert_eq!(
            allocator.alloc_block().expect("block outside first window"),
            70
        );
        let (start, window) = allocator.lookahead_window_for_test();
        assert_eq!(start, 64);
        assert!(window[6], "allocated block should be marked in lookahead");
    }

    #[test]
    fn failed_pair_allocation_rolls_back_lookahead_bits() {
        let cfg = Config {
            block_size: 512,
            block_count: 96,
        };
        let mut used = vec![true; 96];
        used[70] = false;
        let mut allocator = BlockAllocator::from_used(cfg, used).expect("allocator");

        assert_eq!(
            allocator
                .alloc_pair()
                .expect_err("single free block cannot satisfy a pair"),
            Error::NoSpace
        );
        assert_eq!(
            allocator
                .alloc_block()
                .expect("rolled-back block remains free"),
            70
        );
    }

    #[test]
    fn pair_allocation_uses_c_published_metadata_pair_order() {
        let cfg = Config {
            block_size: 512,
            block_count: 8,
        };
        let used = vec![true, true, false, false, false, false, false, false];
        let mut allocator = BlockAllocator::from_used(cfg, used).expect("allocator");

        assert_eq!(
            allocator.alloc_pair().expect("first mounted pair"),
            [2, 3],
            "callers expect the post-compaction pair order stored in C DIRSTRUCT records"
        );
    }

    #[test]
    fn seeded_allocator_starts_scan_at_mount_seed() {
        let cfg = Config {
            block_size: 512,
            block_count: 16,
        };
        let mut used = vec![true; 16];
        used[2] = false;
        used[11] = false;
        let mut allocator =
            BlockAllocator::from_used_with_start(cfg, used, 9).expect("seeded allocator");

        assert_eq!(
            allocator.alloc_block().expect("first block after seed"),
            11,
            "the mount seed should choose the first free block at or after the seeded lookahead start"
        );
        assert_eq!(
            allocator.alloc_block().expect("wrap to earlier free block"),
            2,
            "after exhausting the seeded window the scan wraps around the block device"
        );
    }

    #[test]
    fn bad_block_reservations_survive_rebuild_inputs() {
        let cfg = Config {
            block_size: 512,
            block_count: 8,
        };
        let mut used = vec![false; 8];
        used[0] = true;
        used[1] = true;
        let mut bad = vec![false; 8];
        bad[3] = true;
        let mut allocator =
            BlockAllocator::from_used_with_bad_blocks(cfg, used, 2, &bad).expect("allocator");

        assert_eq!(
            allocator.alloc_block().expect("first non-bad free block"),
            2
        );
        assert_eq!(allocator.alloc_block().expect("skip session bad block"), 4);
    }

    #[test]
    fn reserving_bad_block_updates_current_lookahead() {
        let cfg = Config {
            block_size: 512,
            block_count: 8,
        };
        let mut used = vec![false; 8];
        used[0] = true;
        used[1] = true;
        let mut allocator = BlockAllocator::from_used(cfg, used).expect("allocator");

        allocator.reserve_bad_block(2).expect("reserve bad block");
        assert_eq!(
            allocator.alloc_block().expect("reserved block is skipped"),
            3
        );
    }
}
