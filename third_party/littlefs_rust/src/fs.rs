use alloc::{
    boxed::Box,
    collections::BTreeMap,
    format,
    rc::Rc,
    string::{String, ToString},
    vec::Vec,
};
use core::cell::{Cell, RefCell};

use crate::{
    allocator::BlockAllocator,
    block_device::BlockDevice,
    cache::BlockCache,
    commit::{CommitEntry, MetadataCommitWriter, checked_u10},
    format::{
        LFS_NULL, LFS_TYPE_CREATE, LFS_TYPE_CTZSTRUCT, LFS_TYPE_DELETE, LFS_TYPE_DIR,
        LFS_TYPE_DIRSTRUCT, LFS_TYPE_HARDTAIL, LFS_TYPE_INLINESTRUCT, LFS_TYPE_MOVESTATE,
        LFS_TYPE_REG, LFS_TYPE_SOFTTAIL, LFS_TYPE_SUPERBLOCK, LFS_TYPE_USERATTR, Tag, ctz, le32,
        npw2, popc,
    },
    metadata::{FileData, FileRecord, GlobalState, MetadataPair, MetadataTail, StorageRef},
    path::{components, join_path, normalize_dir_path},
    types::{
        Config, DirEntry, DirectoryUsage, Error, FileType, FilesystemLimits, FilesystemOptions,
        FsInfo, Result, WalkEntry,
    },
    writer::ImageBuilder,
};

mod handles;
mod mutable;
mod read;

const SUPPORTED_DISK_VERSION: u32 = 0x0002_0001;

/// Read-only view over a littlefs disk image or block device.
///
/// Slice-backed mounts are still valuable for fixtures, corruption tests, and
/// C-oracle artifacts that already exist as complete images. The block-device
/// mount path, however, deliberately borrows the device and reads blocks on
/// demand, so normal read-only user semantics do not require copying a whole
/// flash image into RAM.
#[derive(Debug, Clone)]
pub struct Filesystem<'a> {
    image: ImageStorage<'a>,
    cfg: Config,
    root: MetadataPair,
    info: FsInfo,
    options: FilesystemOptions,
    global_state: GlobalState,
    allocation_seed: u32,
}

/// Mutable mounted filesystem shell backed by a real block device.
///
/// The mounted view owns the device and keeps a read-only parser view pointed
/// at that same device through a small block cache. Mounted mutations commit
/// through the block-device/cache layer only. Unsupported native transaction
/// shapes return `Error::Unsupported` instead of falling back to a hidden
/// capacity-sized image allocation.
#[derive(Debug)]
pub struct FilesystemMut<D: BlockDevice + 'static> {
    fs: Filesystem<'static>,
    device: Box<D>,
    cache: BlockCache,
    allocator: BlockAllocator,
    block_cycles: Option<u32>,
}

/// Create-only file writer for the mounted mutable API.
///
/// Small files stay buffered so callers can seek before close. Once sequential
/// writes cross the inline threshold, the handle streams CTZ data blocks before
/// publishing the close-time metadata commit.
pub struct FileWriter<'fs, D: BlockDevice + 'static> {
    fs: &'fs mut FilesystemMut<D>,
    path: String,
    data: Vec<u8>,
    pos: usize,
    stream: Option<StreamingWrite>,
}

// Dirty file handles keep these writeback plans until a flush fully succeeds.
// Cloning lets `FileHandle::flush` attempt the transaction while preserving the
// original plan for retry if a block-device sync fails after bytes were written.
#[derive(Clone)]
struct StreamingWrite {
    allocator: BlockAllocator,
    blocks: Vec<u32>,
    current: Option<StreamingBlock>,
    len: usize,
    target: StreamingTarget,
}

#[derive(Clone)]
struct StreamingBlock {
    block: u32,
    bytes: Vec<u8>,
    off: usize,
    mode: StreamingBlockMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StreamingBlockMode {
    /// A freshly allocated CTZ block. It must be erased before programming and
    /// appended to the logical CTZ block list once durable.
    New,
    /// The last block of an existing CTZ file. Appends may program its erased
    /// tail bytes in place, but the block is already part of `blocks`.
    ExistingTail,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StreamingTarget {
    Create,
    Replace,
}

#[derive(Clone)]
struct MergeWrite {
    original_len: usize,
    patches: Vec<FilePatch>,
}

#[derive(Clone)]
struct FilePatch {
    off: usize,
    data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FileOptions {
    read: bool,
    write: bool,
    create: bool,
    create_new: bool,
    truncate: bool,
    append: bool,
}

/// Option-driven mounted file handle.
///
/// Writable handles buffer only when they need random-write merge semantics.
/// New files, truncating replacements, and CTZ appends can switch to the
/// streaming CTZ writer, which programs data blocks before the exposing
/// metadata commit. Read-only handles keep only the current offset and ask the
/// mounted snapshot to fill caller buffers on demand.
pub struct FileHandle<'fs, D: BlockDevice + 'static> {
    fs: &'fs mut FilesystemMut<D>,
    path: String,
    /// Buffered logical contents for handles that may write, append, or
    /// truncate. Empty for pure read-only streaming handles.
    data: Vec<u8>,
    /// Current logical file position, shared by both the buffered and streaming
    /// paths.
    pos: usize,
    /// Last known logical file length. For buffered writers this tracks
    /// `data.len()`; for streaming readers it records the current `stat`
    /// result so seek/read decisions do not need to preload the file.
    len: usize,
    /// True when reads should be served by `Filesystem::read_file_at` instead
    /// of the buffered `data` vector.
    stream_read: bool,
    /// Immutable source captured when a pure read-only handle is opened.
    /// Holding the resolved file data here avoids repeating path lookup and
    /// metadata folding on every small `read` call.
    stream_source: Option<FileData>,
    /// Streaming mode to use if this handle crosses the CTZ threshold. Missing
    /// files create a new directory entry; existing files replace their current
    /// struct with a new CTZ head after data blocks are durable.
    stream_target: StreamingTarget,
    /// Streaming CTZ state for newly-created files, truncating replacements,
    /// and CTZ append handles.
    stream: Option<StreamingWrite>,
    /// Patch list for write-only partial overwrites of an existing CTZ file.
    /// At flush time the handle streams a replacement CTZ chain by reading old
    /// data in small chunks and overlaying these patches. This is not C's exact
    /// file cache, but it avoids buffering the whole file for the common
    /// "overwrite a range in a large file" shape.
    merge: Option<MergeWrite>,
    readable: bool,
    writable: bool,
    dirty: bool,
}

/// Stream-like directory handle for the public read API.
///
/// The handle borrows the mounted filesystem and stores only a directory head
/// plus cursor position. Reads fold metadata on demand instead of keeping an
/// owned `Vec<DirEntry>` snapshot, which keeps large directory handles bounded
/// by metadata scratch state rather than entry count.
pub struct DirHandle<'fs, 'a> {
    fs: &'fs Filesystem<'a>,
    head: [u32; 2],
    pos: usize,
}

enum ImageStorage<'a> {
    Borrowed(&'a [u8]),
    Owned(Vec<u8>),
    Device {
        device: &'a dyn BlockDevice,
        cache: Rc<ReadBlockCache>,
    },
}

impl core::fmt::Debug for ImageStorage<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Borrowed(image) => f
                .debug_tuple("Borrowed")
                .field(&format_args!("{} bytes", image.len()))
                .finish(),
            Self::Owned(image) => f
                .debug_tuple("Owned")
                .field(&format_args!("{} bytes", image.len()))
                .finish(),
            Self::Device { .. } => f.debug_tuple("Device").field(&"<block-device>").finish(),
        }
    }
}

impl<'a> Clone for ImageStorage<'a> {
    fn clone(&self) -> Self {
        match self {
            Self::Borrowed(image) => Self::Borrowed(*image),
            Self::Owned(image) => Self::Owned(image.clone()),
            Self::Device { device, cache } => Self::Device {
                device: *device,
                cache: cache.clone(),
            },
        }
    }
}

#[derive(Debug)]
struct ReadBlockCache {
    slots: RefCell<Vec<ReadBlockCacheSlot>>,
    next: Cell<usize>,
    chunk_size: usize,
}

#[derive(Debug)]
struct ReadBlockCacheSlot {
    block: Option<u32>,
    off: usize,
    len: usize,
    data: Vec<u8>,
}

impl ReadBlockCache {
    fn new(cache_size: usize, slots: usize) -> Self {
        let slots = core::cmp::max(slots, 1);
        let cache_size = core::cmp::max(cache_size, 1);
        Self {
            slots: RefCell::new(
                (0..slots)
                    .map(|_| ReadBlockCacheSlot {
                        block: None,
                        off: 0,
                        len: 0,
                        data: alloc::vec![0xff; cache_size],
                    })
                    .collect(),
            ),
            next: Cell::new(0),
            chunk_size: cache_size,
        }
    }

    fn read(
        &self,
        device: &dyn BlockDevice,
        cfg: Config,
        block: u32,
        off: usize,
        out: &mut [u8],
    ) -> Result<()> {
        if off.checked_add(out.len()).ok_or(Error::OutOfBounds)? > cfg.block_size {
            return Err(Error::OutOfBounds);
        }

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

    fn read_chunk(
        &self,
        device: &dyn BlockDevice,
        block: u32,
        off: usize,
        chunk_len: usize,
        in_chunk: usize,
        out: &mut [u8],
    ) -> Result<()> {
        let mut slots = self.slots.borrow_mut();
        if let Some(slot) = slots
            .iter()
            .find(|slot| slot.block == Some(block) && slot.off == off && slot.len == chunk_len)
        {
            out.copy_from_slice(&slot.data[in_chunk..in_chunk + out.len()]);
            return Ok(());
        }

        let slot_index = self.next.get() % slots.len();
        self.next.set((slot_index + 1) % slots.len());
        let slot = &mut slots[slot_index];
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
}

fn ctz_data_start(index: usize) -> Result<usize> {
    if index == 0 {
        Ok(0)
    } else {
        let skips = index.trailing_zeros() as usize + 1;
        skips.checked_mul(4).ok_or(Error::NoSpace)
    }
}

fn program_nor_bytes(block: &mut [u8], off: usize, data: &[u8]) -> Result<()> {
    let end = off.checked_add(data.len()).ok_or(Error::NoSpace)?;
    if end > block.len() {
        return Err(Error::NoSpace);
    }
    for (dst, src) in block[off..end].iter_mut().zip(data) {
        *dst &= *src;
    }
    Ok(())
}

fn root_create_id(files: &[FileRecord], name: &str) -> Result<u16> {
    let id = files
        .iter()
        .filter(|file| file.name.as_str() < name)
        .count()
        .checked_add(1)
        .ok_or(Error::Unsupported)?;
    let id = u16::try_from(id).map_err(|_| Error::Unsupported)?;
    if id < 0x3ff {
        Ok(id)
    } else {
        Err(Error::Unsupported)
    }
}

fn dir_create_id(files: &[FileRecord], name: &str) -> Result<u16> {
    let id = files
        .iter()
        .filter(|file| file.name.as_str() < name)
        .count();
    let id = u16::try_from(id).map_err(|_| Error::Unsupported)?;
    if id < 0x3ff {
        Ok(id)
    } else {
        Err(Error::Unsupported)
    }
}
