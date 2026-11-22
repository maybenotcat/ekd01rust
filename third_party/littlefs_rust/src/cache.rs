use alloc::vec::Vec;

use crate::{
    block_device::BlockDevice,
    types::{Config, Error, Result},
};

/// Minimal block cache used by the mounted writer.
///
/// This is intentionally smaller than littlefs's final cache story, but it
/// establishes the key ownership boundary: filesystem code asks for cached
/// reads, buffered programs, erases, and syncs instead of touching the block
/// device directly. The program cache flushes whenever an operation switches to
/// a different block, which preserves the explicit data-before-metadata order
/// in the current write paths while still giving us one place to evolve toward
/// prog-size aggregation and streaming file handles.
#[derive(Debug, Clone)]
pub(crate) struct BlockCache {
    cfg: Config,
    read: ChunkReadCache,
    prog: CacheSlot,
}

#[derive(Debug, Clone)]
struct CacheSlot {
    block: Option<u32>,
    off: usize,
    data: Vec<u8>,
    dirty: bool,
}

#[derive(Debug, Clone)]
struct ChunkReadCache {
    slots: Vec<ReadCacheSlot>,
    next: usize,
    chunk_size: usize,
}

#[derive(Debug, Clone)]
struct ReadCacheSlot {
    block: Option<u32>,
    off: usize,
    len: usize,
    data: Vec<u8>,
}

impl BlockCache {
    pub(crate) fn new_with_cache_size(cfg: Config, cache_size: usize) -> Result<Self> {
        if cfg.block_size == 0 || cfg.block_count == 0 {
            return Err(Error::InvalidConfig);
        }
        Ok(Self {
            cfg,
            read: ChunkReadCache::new(cache_size, 2),
            prog: CacheSlot::new(),
        })
    }

    pub(crate) fn read<D: BlockDevice>(
        &mut self,
        device: &D,
        block: u32,
        off: usize,
        out: &mut [u8],
    ) -> Result<()> {
        self.check_range(block, off, out.len())?;
        self.read.read(device, self.cfg, block, off, out)?;
        if self.prog.dirty && self.prog.block == Some(block) {
            self.prog.overlay_read(off, out)?;
        }
        Ok(())
    }

    pub(crate) fn prog<D: BlockDevice>(
        &mut self,
        device: &mut D,
        block: u32,
        off: usize,
        data: &[u8],
    ) -> Result<()> {
        self.check_range(block, off, data.len())?;
        if self.prog.block != Some(block) {
            self.flush(device)?;
            self.prog.block = Some(block);
            self.prog.dirty = false;
        }

        self.prog.merge_prog(off, data)?;
        self.read.invalidate_block(block);
        Ok(())
    }

    pub(crate) fn erase<D: BlockDevice>(&mut self, device: &mut D, block: u32) -> Result<()> {
        self.check_range(block, 0, self.cfg.block_size)?;
        self.flush(device)?;
        device.erase(block)?;

        self.read.invalidate_block(block);
        if self.prog.block == Some(block) {
            self.prog.clear();
        }
        Ok(())
    }

    pub(crate) fn sync<D: BlockDevice>(&mut self, device: &mut D) -> Result<()> {
        self.flush(device)?;
        device.sync()
    }

    pub(crate) fn invalidate_all(&mut self) {
        self.read.invalidate_all();
        self.prog.clear();
    }

    fn flush<D: BlockDevice>(&mut self, device: &mut D) -> Result<()> {
        if self.prog.dirty {
            let block = self.prog.block.ok_or(Error::Corrupt)?;
            device.prog(block, self.prog.off, &self.prog.data)?;
            self.read.invalidate_block(block);
            self.prog.clear();
        }
        Ok(())
    }

    fn check_range(&self, block: u32, off: usize, len: usize) -> Result<()> {
        if block as usize >= self.cfg.block_count {
            return Err(Error::OutOfBounds);
        }
        let end = off.checked_add(len).ok_or(Error::OutOfBounds)?;
        if end > self.cfg.block_size {
            return Err(Error::OutOfBounds);
        }
        Ok(())
    }
}

impl ChunkReadCache {
    fn new(chunk_size: usize, slots: usize) -> Self {
        let chunk_size = core::cmp::max(chunk_size, 1);
        let slots = core::cmp::max(slots, 1);
        Self {
            slots: (0..slots)
                .map(|_| ReadCacheSlot {
                    block: None,
                    off: 0,
                    len: 0,
                    data: alloc::vec![0xff; chunk_size],
                })
                .collect(),
            next: 0,
            chunk_size,
        }
    }

    fn read<D: BlockDevice>(
        &mut self,
        device: &D,
        cfg: Config,
        block: u32,
        off: usize,
        out: &mut [u8],
    ) -> Result<()> {
        let mut copied = 0usize;
        while copied < out.len() {
            let absolute = off + copied;
            let chunk_off = (absolute / self.chunk_size) * self.chunk_size;
            let chunk_len = core::cmp::min(self.chunk_size, cfg.block_size - chunk_off);
            let in_chunk = absolute - chunk_off;
            let len = core::cmp::min(out.len() - copied, chunk_len - in_chunk);
            self.read_chunk(
                device,
                block,
                chunk_off,
                chunk_len,
                in_chunk,
                &mut out[copied..copied + len],
            )?;
            copied += len;
        }
        Ok(())
    }

    fn read_chunk<D: BlockDevice>(
        &mut self,
        device: &D,
        block: u32,
        off: usize,
        chunk_len: usize,
        in_chunk: usize,
        out: &mut [u8],
    ) -> Result<()> {
        if let Some(slot) = self
            .slots
            .iter()
            .find(|slot| slot.block == Some(block) && slot.off == off && slot.len == chunk_len)
        {
            out.copy_from_slice(&slot.data[in_chunk..in_chunk + out.len()]);
            return Ok(());
        }

        let slot_index = self.next % self.slots.len();
        self.next = (slot_index + 1) % self.slots.len();
        let slot = &mut self.slots[slot_index];
        if slot.data.len() < chunk_len {
            slot.data.resize(chunk_len, 0xff);
        }
        device.read(block, off, &mut slot.data[..chunk_len])?;
        slot.block = Some(block);
        slot.off = off;
        slot.len = chunk_len;
        out.copy_from_slice(&slot.data[in_chunk..in_chunk + out.len()]);
        Ok(())
    }

    fn invalidate_block(&mut self, block: u32) {
        for slot in &mut self.slots {
            if slot.block == Some(block) {
                slot.block = None;
                slot.len = 0;
            }
        }
    }

    fn invalidate_all(&mut self) {
        for slot in &mut self.slots {
            slot.block = None;
            slot.len = 0;
        }
    }
}

impl CacheSlot {
    fn new() -> Self {
        Self {
            block: None,
            off: 0,
            data: Vec::new(),
            dirty: false,
        }
    }

    fn merge_prog(&mut self, off: usize, data: &[u8]) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }

        if !self.dirty {
            self.off = off;
            self.data.clear();
            self.data.extend_from_slice(data);
            self.dirty = true;
            return Ok(());
        }

        let old_start = self.off;
        let old_end = self
            .off
            .checked_add(self.data.len())
            .ok_or(Error::OutOfBounds)?;
        let new_end = off.checked_add(data.len()).ok_or(Error::OutOfBounds)?;
        let merged_start = core::cmp::min(old_start, off);
        let merged_end = core::cmp::max(old_end, new_end);
        let mut merged = alloc::vec![0xff; merged_end - merged_start];

        let old_offset = old_start - merged_start;
        merged[old_offset..old_offset + self.data.len()].copy_from_slice(&self.data);

        let new_offset = off - merged_start;
        for (dst, src) in merged[new_offset..new_offset + data.len()]
            .iter_mut()
            .zip(data)
        {
            *dst &= *src;
        }

        self.off = merged_start;
        self.data = merged;
        self.dirty = true;
        Ok(())
    }

    fn overlay_read(&self, read_off: usize, out: &mut [u8]) -> Result<()> {
        if !self.dirty || out.is_empty() {
            return Ok(());
        }
        let dirty_start = self.off;
        let dirty_end = self
            .off
            .checked_add(self.data.len())
            .ok_or(Error::OutOfBounds)?;
        let read_end = read_off.checked_add(out.len()).ok_or(Error::OutOfBounds)?;
        let start = core::cmp::max(dirty_start, read_off);
        let end = core::cmp::min(dirty_end, read_end);
        if start >= end {
            return Ok(());
        }
        let out_start = start - read_off;
        let dirty_offset = start - dirty_start;
        for (dst, mask) in out[out_start..out_start + (end - start)]
            .iter_mut()
            .zip(&self.data[dirty_offset..dirty_offset + (end - start)])
        {
            *dst &= *mask;
        }
        Ok(())
    }

    fn clear(&mut self) {
        self.block = None;
        self.off = 0;
        self.dirty = false;
        self.data.clear();
        self.data.shrink_to_fit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_device::MemoryBlockDevice;

    #[test]
    fn read_observes_buffered_prog_before_sync() {
        let cfg = Config {
            block_size: 16,
            block_count: 4,
        };
        let mut device = MemoryBlockDevice::new_erased(cfg).expect("memory device");
        let mut cache = BlockCache::new_with_cache_size(cfg, cfg.cache_size()).expect("cache");

        cache.prog(&mut device, 2, 4, b"rust").expect("cached prog");
        let mut cached = [0; 4];
        cache
            .read(&device, 2, 4, &mut cached)
            .expect("read through cache");
        assert_eq!(&cached, b"rust");

        let mut raw = [0xff; 4];
        device
            .read(2, 4, &mut raw)
            .expect("raw device is still unflushed");
        assert_eq!(raw, [0xff; 4]);

        cache.sync(&mut device).expect("sync cached prog");
        device.read(2, 4, &mut raw).expect("raw device after sync");
        assert_eq!(&raw, b"rust");
    }

    #[test]
    fn erase_invalidates_cached_read_data() {
        let cfg = Config {
            block_size: 16,
            block_count: 4,
        };
        let mut device = MemoryBlockDevice::new_erased(cfg).expect("memory device");
        device.prog(1, 0, b"old").expect("seed block");
        let mut cache = BlockCache::new_with_cache_size(cfg, cfg.cache_size()).expect("cache");

        let mut before = [0; 3];
        cache
            .read(&device, 1, 0, &mut before)
            .expect("fill read cache");
        assert_eq!(&before, b"old");

        cache.erase(&mut device, 1).expect("erase through cache");
        let mut after = [0; 3];
        cache
            .read(&device, 1, 0, &mut after)
            .expect("read erased block");
        assert_eq!(after, [0xff; 3]);
    }
}
