use alloc::{boxed::Box, vec::Vec};

use crate::types::{Config, Error, Result};

#[cfg(feature = "std")]
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    sync::Mutex,
};

/// Minimal synchronous block-device interface for littlefs experiments.
///
/// The trait deliberately mirrors littlefs's block-level vocabulary instead of
/// exposing a raw byte slice. `read` and `prog` operate inside one logical
/// block, `erase` resets an entire block, and `sync` gives later writable
/// backends a place to flush durable state.
pub trait BlockDevice {
    fn config(&self) -> Config;

    fn read(&self, block: u32, off: usize, out: &mut [u8]) -> Result<()>;

    fn prog(&mut self, block: u32, off: usize, data: &[u8]) -> Result<()>;

    fn erase(&mut self, block: u32) -> Result<()>;

    fn sync(&mut self) -> Result<()> {
        Ok(())
    }
}

/// In-memory NOR-flash block device used by tests and early integrations.
///
/// Blocks start erased to `0xff`. Programming uses `old & new`, so accidental
/// attempts to turn a programmed zero bit back into one are visible in the
/// stored bytes just as they would be on NOR flash. This is intentionally a
/// real block device implementation for tests, not a mock of filesystem
/// behavior.
#[derive(Debug, Clone)]
pub struct MemoryBlockDevice {
    cfg: Config,
    storage: Vec<u8>,
}

/// File-backed block device for black-box tests and desktop experiments.
///
/// This backend performs direct local-file reads and writes. `sync` forwards to
/// the host file's data-sync operation so file-handle flush/sync tests exercise
/// a real backend boundary. Like `MemoryBlockDevice`, `prog` preserves NOR
/// semantics by reading the current bytes, applying `old & new`, and writing
/// the result back to the file.
#[cfg(feature = "std")]
#[derive(Debug)]
pub struct FileBlockDevice {
    cfg: Config,
    file: Mutex<File>,
}

impl MemoryBlockDevice {
    pub fn new_erased(cfg: Config) -> Result<Self> {
        let len = image_len(cfg)?;
        Ok(Self {
            cfg,
            storage: alloc::vec![0xff; len],
        })
    }

    pub fn from_bytes(cfg: Config, bytes: &[u8]) -> Result<Self> {
        let len = image_len(cfg)?;
        if bytes.len() != len {
            return Err(Error::InvalidConfig);
        }
        Ok(Self {
            cfg,
            storage: bytes.to_vec(),
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.storage
    }

    fn block_range(&self, block: u32, off: usize, len: usize) -> Result<core::ops::Range<usize>> {
        let block = block as usize;
        if block >= self.cfg.block_count {
            return Err(Error::OutOfBounds);
        }
        let end_off = off.checked_add(len).ok_or(Error::OutOfBounds)?;
        if end_off > self.cfg.block_size {
            return Err(Error::OutOfBounds);
        }
        let start = block
            .checked_mul(self.cfg.block_size)
            .and_then(|base| base.checked_add(off))
            .ok_or(Error::OutOfBounds)?;
        let end = start.checked_add(len).ok_or(Error::OutOfBounds)?;
        Ok(start..end)
    }
}

#[cfg(feature = "std")]
impl FileBlockDevice {
    pub fn create_erased<P: AsRef<Path>>(path: P, cfg: Config) -> Result<Self> {
        let len = image_len(cfg)?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .map_err(|_| Error::Io)?;

        let erased = alloc::vec![0xff; cfg.block_size];
        for _ in 0..cfg.block_count {
            file.write_all(&erased).map_err(|_| Error::Io)?;
        }
        file.set_len(len as u64).map_err(|_| Error::Io)?;
        Ok(Self {
            cfg,
            file: Mutex::new(file),
        })
    }

    pub fn open<P: AsRef<Path>>(path: P, cfg: Config) -> Result<Self> {
        let len = image_len(cfg)? as u64;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|_| Error::Io)?;
        if file.metadata().map_err(|_| Error::Io)?.len() != len {
            return Err(Error::InvalidConfig);
        }
        Ok(Self {
            cfg,
            file: Mutex::new(file),
        })
    }

    pub fn from_bytes<P: AsRef<Path>>(path: P, cfg: Config, bytes: &[u8]) -> Result<Self> {
        let len = image_len(cfg)?;
        if bytes.len() != len {
            return Err(Error::InvalidConfig);
        }

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .map_err(|_| Error::Io)?;
        file.write_all(bytes).map_err(|_| Error::Io)?;
        Ok(Self {
            cfg,
            file: Mutex::new(file),
        })
    }

    fn block_range(&self, block: u32, off: usize, len: usize) -> Result<core::ops::Range<usize>> {
        let block = block as usize;
        if block >= self.cfg.block_count {
            return Err(Error::OutOfBounds);
        }
        let end_off = off.checked_add(len).ok_or(Error::OutOfBounds)?;
        if end_off > self.cfg.block_size {
            return Err(Error::OutOfBounds);
        }
        let start = block
            .checked_mul(self.cfg.block_size)
            .and_then(|base| base.checked_add(off))
            .ok_or(Error::OutOfBounds)?;
        let end = start.checked_add(len).ok_or(Error::OutOfBounds)?;
        Ok(start..end)
    }
}

impl BlockDevice for MemoryBlockDevice {
    fn config(&self) -> Config {
        self.cfg
    }

    fn read(&self, block: u32, off: usize, out: &mut [u8]) -> Result<()> {
        let range = self.block_range(block, off, out.len())?;
        out.copy_from_slice(&self.storage[range]);
        Ok(())
    }

    fn prog(&mut self, block: u32, off: usize, data: &[u8]) -> Result<()> {
        let range = self.block_range(block, off, data.len())?;
        for (dst, src) in self.storage[range].iter_mut().zip(data) {
            *dst &= *src;
        }
        Ok(())
    }

    fn erase(&mut self, block: u32) -> Result<()> {
        let range = self.block_range(block, 0, self.cfg.block_size)?;
        self.storage[range].fill(0xff);
        Ok(())
    }
}

impl<D: BlockDevice + ?Sized> BlockDevice for Box<D> {
    fn config(&self) -> Config {
        (**self).config()
    }

    fn read(&self, block: u32, off: usize, out: &mut [u8]) -> Result<()> {
        (**self).read(block, off, out)
    }

    fn prog(&mut self, block: u32, off: usize, data: &[u8]) -> Result<()> {
        (**self).prog(block, off, data)
    }

    fn erase(&mut self, block: u32) -> Result<()> {
        (**self).erase(block)
    }

    fn sync(&mut self) -> Result<()> {
        (**self).sync()
    }
}

#[cfg(feature = "std")]
impl BlockDevice for FileBlockDevice {
    fn config(&self) -> Config {
        self.cfg
    }

    fn read(&self, block: u32, off: usize, out: &mut [u8]) -> Result<()> {
        let range = self.block_range(block, off, out.len())?;
        let mut file = self.file.lock().map_err(|_| Error::Io)?;
        file.seek(SeekFrom::Start(range.start as u64))
            .map_err(|_| Error::Io)?;
        file.read_exact(out).map_err(|_| Error::Io)
    }

    fn prog(&mut self, block: u32, off: usize, data: &[u8]) -> Result<()> {
        let range = self.block_range(block, off, data.len())?;
        let mut file = self.file.lock().map_err(|_| Error::Io)?;
        let chunk_size = self.cfg.cache_size();
        let mut old = alloc::vec![0xff; chunk_size];
        let mut copied = 0usize;
        while copied < data.len() {
            let n = core::cmp::min(chunk_size, data.len() - copied);
            let file_off = range.start + copied;
            file.seek(SeekFrom::Start(file_off as u64))
                .map_err(|_| Error::Io)?;
            file.read_exact(&mut old[..n]).map_err(|_| Error::Io)?;
            for (dst, src) in old[..n].iter_mut().zip(&data[copied..copied + n]) {
                *dst &= *src;
            }
            file.seek(SeekFrom::Start(file_off as u64))
                .map_err(|_| Error::Io)?;
            file.write_all(&old[..n]).map_err(|_| Error::Io)?;
            copied += n;
        }
        Ok(())
    }

    fn erase(&mut self, block: u32) -> Result<()> {
        let range = self.block_range(block, 0, self.cfg.block_size)?;
        let mut file = self.file.lock().map_err(|_| Error::Io)?;
        let chunk_size = self.cfg.cache_size();
        let erased = alloc::vec![0xff; chunk_size];
        let mut copied = 0usize;
        while copied < self.cfg.block_size {
            let n = core::cmp::min(chunk_size, self.cfg.block_size - copied);
            file.seek(SeekFrom::Start((range.start + copied) as u64))
                .map_err(|_| Error::Io)?;
            file.write_all(&erased[..n]).map_err(|_| Error::Io)?;
            copied += n;
        }
        Ok(())
    }

    fn sync(&mut self) -> Result<()> {
        let file = self.file.lock().map_err(|_| Error::Io)?;
        file.sync_data().map_err(|_| Error::Io)
    }
}

fn image_len(cfg: Config) -> Result<usize> {
    if cfg.block_size == 0 || cfg.block_count == 0 {
        return Err(Error::InvalidConfig);
    }
    cfg.block_size
        .checked_mul(cfg.block_count)
        .ok_or(Error::InvalidConfig)
}
