use ::alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec,
    vec::Vec,
};

mod alloc;
mod block;
mod builder;
mod commits;
mod ctz;
mod editor;
mod tree;

use self::{alloc::FreshAllocator, block::image_block_mut, ctz::CtzFile};
use crate::{
    commit::{CommitEntry, CommitState, MetadataCommitWriter, checked_u10},
    format::{
        LFS_TYPE_CREATE, LFS_TYPE_CTZSTRUCT, LFS_TYPE_DELETE, LFS_TYPE_DIR, LFS_TYPE_DIRSTRUCT,
        LFS_TYPE_INLINESTRUCT, LFS_TYPE_REG, LFS_TYPE_SUPERBLOCK, LFS_TYPE_USERATTR, Tag,
    },
    fs::Filesystem,
    metadata::{FileData, MetadataPair},
    path::components,
    types::{Config, Error, FilesystemOptions, Result},
};

/// Disk version written by the current upstream littlefs release in this repo.
///
/// Pinning this value in one place matters because littlefs may decide to
/// rewrite older superblocks during mount. C-oracle tests should inspect the
/// Rust-written bytes directly, not an upgraded image rewritten by C.
const DISK_VERSION: u32 = 0x0002_0001;

const DEFAULT_NAME_MAX: u32 = 255;
const DEFAULT_ATTR_MAX: u32 = 1_022;
const METADATA_PROG_SIZE: usize = 16;

/// Explicit full-image builder for host-side fixtures and interop tests.
///
/// Mounted block-device users should prefer `Filesystem::format_device` and
/// `Filesystem::mount_device_mut`. This builder is still useful when a caller
/// already wants a complete image in memory.
#[derive(Debug, Clone)]
pub struct ImageBuilder {
    cfg: Config,
    options: FilesystemOptions,
    entries: BTreeMap<String, RootEntry>,
    visible_entries: BTreeMap<String, RootKind>,
    update_commits: Vec<RootUpdateCommit>,
    allocator: FreshAllocator,
}

/// Explicit full-image editor for already materialized littlefs images.
///
/// This is an offline/host-side utility. Mounted APIs must not call it as a
/// hidden fallback because it owns a complete image `Vec`.
#[derive(Debug, Clone)]
pub struct ImageEditor {
    cfg: Config,
    image: Vec<u8>,
    root: MetadataPair,
    used_blocks: Vec<bool>,
}

#[derive(Debug, Clone)]
enum RootEdit {
    Storage {
        id: u16,
        storage: FileStorage,
    },
    Attr {
        id: u16,
        attr_type: u8,
        value: Option<Vec<u8>>,
    },
    Delete {
        id: u16,
    },
}

#[derive(Debug, Clone)]
enum RootEntry {
    File(InlineFile),
    Dir(Directory),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootKind {
    File,
    Dir,
}

#[derive(Debug, Clone)]
struct InlineFile {
    storage: FileStorage,
    attrs: BTreeMap<u8, Vec<u8>>,
}

#[derive(Debug, Clone)]
enum FileStorage {
    Inline(Vec<u8>),
    Ctz(CtzFile),
    ExistingCtz { head: u32, size: u32 },
}

#[derive(Debug, Clone)]
struct Directory {
    pair: [u32; 2],
    cfg: Config,
    options: FilesystemOptions,
    entries: BTreeMap<String, DirectoryEntry>,
    visible_entries: BTreeMap<String, RootKind>,
    update_commits: Vec<DirUpdateCommit>,
}

#[derive(Debug, Clone)]
enum DirectoryEntry {
    File(InlineFile),
    Dir(Directory),
}

#[derive(Debug, Clone)]
struct DirUpdateCommit {
    id: u16,
    storage: Option<FileStorage>,
    attrs: BTreeMap<u8, Option<Vec<u8>>>,
    delete_file: bool,
}

#[derive(Debug, Clone)]
struct RootUpdateCommit {
    id: u16,
    storage: Option<FileStorage>,
    attrs: BTreeMap<u8, Option<Vec<u8>>>,
    delete_file: bool,
}

#[derive(Debug)]
struct RootCommit {
    entries: Vec<CommitEntry>,
}

fn superblock_payload(cfg: Config, options: FilesystemOptions) -> Vec<u8> {
    let mut payload = Vec::with_capacity(24);
    for word in [
        DISK_VERSION,
        cfg.block_size as u32,
        cfg.block_count as u32,
        options.name_max,
        options.file_max,
        options.attr_max,
    ] {
        payload.extend_from_slice(&word.to_le_bytes());
    }
    payload
}

fn storage_struct_entry(id: u16, storage: &FileStorage) -> Result<CommitEntry> {
    match storage {
        FileStorage::Inline(data) => Ok(CommitEntry::new(
            Tag::new(LFS_TYPE_INLINESTRUCT, id, checked_u10(data.len())?),
            data,
        )),
        FileStorage::Ctz(ctz) => {
            let mut payload = Vec::with_capacity(8);
            payload.extend_from_slice(&ctz.head()?.to_le_bytes());
            payload.extend_from_slice(&(ctz.len() as u32).to_le_bytes());
            Ok(CommitEntry::new(
                Tag::new(LFS_TYPE_CTZSTRUCT, id, 8),
                &payload,
            ))
        }
        FileStorage::ExistingCtz { head, size } => {
            let mut payload = Vec::with_capacity(8);
            payload.extend_from_slice(&head.to_le_bytes());
            payload.extend_from_slice(&size.to_le_bytes());
            Ok(CommitEntry::new(
                Tag::new(LFS_TYPE_CTZSTRUCT, id, 8),
                &payload,
            ))
        }
    }
}

fn split_parent<'a>(parts: &'a [&'a str]) -> Result<(&'a str, &'a [&'a str])> {
    let (name, parents) = parts.split_last().ok_or(Error::Unsupported)?;
    Ok((*name, parents))
}

fn root_entry_id<T>(entries: &BTreeMap<String, T>, name: &str) -> Result<u16> {
    // Root entry ids are assigned by lexical order after the superblock's
    // permanent id 0. Updates must reuse that same id; otherwise C would attach
    // the new struct/attr to a different directory entry.
    for (index, existing) in entries.keys().enumerate() {
        if existing == name {
            let id = u16::try_from(index + 1).map_err(|_| Error::Unsupported)?;
            return if id < 0x3ff {
                Ok(id)
            } else {
                Err(Error::Unsupported)
            };
        }
    }
    Err(Error::NotFound)
}

fn root_key_for_id<T>(entries: &BTreeMap<String, T>, id: u16) -> Result<&str> {
    if id == 0 {
        return Err(Error::Unsupported);
    }
    entries
        .keys()
        .nth(id as usize - 1)
        .map(|key| key.as_str())
        .ok_or(Error::Corrupt)
}

fn root_create_id(files: &[crate::metadata::FileRecord], name: &str) -> Result<u16> {
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

fn dir_create_id(files: &[crate::metadata::FileRecord], name: &str) -> Result<u16> {
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

fn dir_key_for_id<T>(entries: &BTreeMap<String, T>, id: u16) -> Result<&str> {
    entries
        .keys()
        .nth(id as usize)
        .map(|key| key.as_str())
        .ok_or(Error::Corrupt)
}

fn directory_entries(
    entries_by_name: &BTreeMap<String, DirectoryEntry>,
) -> Result<Vec<CommitEntry>> {
    let mut entries = Vec::new();
    for (index, (name, entry)) in entries_by_name.iter().enumerate() {
        let id = u16::try_from(index).map_err(|_| Error::Unsupported)?;
        if id >= 0x3ff {
            return Err(Error::Unsupported);
        }
        entries.push(CommitEntry::new(Tag::new(LFS_TYPE_CREATE, id, 0), &[]));
        match entry {
            DirectoryEntry::File(file) => {
                entries.push(CommitEntry::new(
                    Tag::new(LFS_TYPE_REG, id, checked_u10(name.len())?),
                    name.as_bytes(),
                ));
                match &file.storage {
                    FileStorage::Inline(data) => {
                        entries.push(CommitEntry::new(
                            Tag::new(LFS_TYPE_INLINESTRUCT, id, checked_u10(data.len())?),
                            data,
                        ));
                    }
                    FileStorage::Ctz(ctz) => {
                        let mut payload = Vec::with_capacity(8);
                        payload.extend_from_slice(&ctz.head()?.to_le_bytes());
                        payload.extend_from_slice(&(ctz.len() as u32).to_le_bytes());
                        entries.push(CommitEntry::new(
                            Tag::new(LFS_TYPE_CTZSTRUCT, id, 8),
                            &payload,
                        ));
                    }
                    FileStorage::ExistingCtz { head, size } => {
                        let mut payload = Vec::with_capacity(8);
                        payload.extend_from_slice(&head.to_le_bytes());
                        payload.extend_from_slice(&size.to_le_bytes());
                        entries.push(CommitEntry::new(
                            Tag::new(LFS_TYPE_CTZSTRUCT, id, 8),
                            &payload,
                        ));
                    }
                }
                for (attr_type, attr) in &file.attrs {
                    entries.push(CommitEntry::new(
                        Tag::new(
                            LFS_TYPE_USERATTR + u16::from(*attr_type),
                            id,
                            checked_u10(attr.len())?,
                        ),
                        attr,
                    ));
                }
            }
            DirectoryEntry::Dir(dir) => {
                entries.push(CommitEntry::new(
                    Tag::new(LFS_TYPE_DIR, id, checked_u10(name.len())?),
                    name.as_bytes(),
                ));
                let mut pair = Vec::with_capacity(8);
                pair.extend_from_slice(&dir.pair[0].to_le_bytes());
                pair.extend_from_slice(&dir.pair[1].to_le_bytes());
                entries.push(CommitEntry::new(Tag::new(LFS_TYPE_DIRSTRUCT, id, 8), &pair));
            }
        }
    }
    Ok(entries)
}

fn child_file_id(entries: &BTreeMap<String, RootKind>, name: &str) -> Result<u16> {
    for (index, (existing, kind)) in entries.iter().enumerate() {
        if existing == name {
            if *kind != RootKind::File {
                return Err(Error::Unsupported);
            }
            let id = u16::try_from(index).map_err(|_| Error::Unsupported)?;
            return if id < 0x3ff {
                Ok(id)
            } else {
                Err(Error::Unsupported)
            };
        }
    }
    Err(Error::NotFound)
}

fn child_dir_id(entries: &BTreeMap<String, RootKind>, name: &str) -> Result<u16> {
    for (index, (existing, kind)) in entries.iter().enumerate() {
        if existing == name {
            if *kind != RootKind::Dir {
                return Err(Error::Unsupported);
            }
            let id = u16::try_from(index).map_err(|_| Error::Unsupported)?;
            return if id < 0x3ff {
                Ok(id)
            } else {
                Err(Error::Unsupported)
            };
        }
    }
    Err(Error::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filesystem;

    #[test]
    fn built_image_mounts_with_rust_reader() {
        let mut builder = ImageBuilder::new(Config {
            block_size: 512,
            block_count: 64,
        })
        .expect("builder");
        builder
            .add_inline_file("/hello.txt", b"hello from rust\n")
            .expect("add file")
            .set_attr("/hello.txt", 0x42, b"greeting")
            .expect("set attr");

        let image = builder.build().expect("build image");
        let fs = Filesystem::mount(
            &image,
            Config {
                block_size: 512,
                block_count: 64,
            },
        )
        .expect("mount generated image");

        assert_eq!(
            fs.read_file("/hello.txt").expect("read generated file"),
            b"hello from rust\n"
        );
        assert_eq!(
            fs.read_attr("/hello.txt", 0x42)
                .expect("read generated attr"),
            b"greeting"
        );
    }
}
