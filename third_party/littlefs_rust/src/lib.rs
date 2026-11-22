#![cfg_attr(not(feature = "std"), no_std)]

//! Pure Rust implementation of the littlefs v2 on-disk format.
//!
//! Inspired by littlefs. This is independent Rust code with interoperability
//! against upstream littlefs treated as a design constraint.
//!
//! The public API is centered on a synchronous [`BlockDevice`] trait plus
//! mounted read-only and mutable filesystem handles. Core library code supports
//! `no_std + alloc`; the default `std` feature adds [`FileBlockDevice`] for
//! local-file backed images in tests and host-side tools.
//!
//! # Example
//!
//! ```no_run
//! use littlefs_rust::{Config, FileBlockDevice, FileOptions, Filesystem};
//!
//! fn main() -> littlefs_rust::Result<()> {
//!     let cfg = Config {
//!         block_size: 512,
//!         block_count: 4096,
//!     };
//!
//!     let path = std::env::temp_dir().join("littlefs-rust-doc-example.img");
//!     let mut device = FileBlockDevice::create_erased(&path, cfg)?;
//!     Filesystem::format_device(&mut device)?;
//!
//!     let mut fs = Filesystem::mount_device_mut(device)?;
//!     fs.create_dir("/logs")?;
//!
//!     let mut file = fs.open_file(
//!         "/logs/current.txt",
//!         FileOptions::new()
//!             .create(true)
//!             .write(true)
//!             .append(true),
//!     )?;
//!     file.write_all(b"boot ok\n")?;
//!     file.sync()?;
//!     file.close()?;
//!
//!     let device = fs.into_device();
//!     let ro = Filesystem::mount_device(&device)?;
//!     assert_eq!(ro.read_file("/logs/current.txt")?, b"boot ok\n");
//!
//!     let _ = std::fs::remove_file(path);
//!     Ok(())
//! }
//! ```
//!
//! # Main Entry Points
//!
//! - [`Filesystem::format_device`] formats a block device.
//! - [`Filesystem::mount_device`] mounts a borrowed block device read-only.
//! - [`Filesystem::mount_device_mut`] mounts an owned block device for writes.
//! - [`FilesystemOptions`] configures littlefs-style cache, program, limit,
//!   block-cycle, and inline-file policy.
//! - [`FileOptions`] configures open-file handles for read, write, append,
//!   create, create-new, and truncate behavior.
//!
//! # Interoperability
//!
//! The repository's default test suite uses real image bytes and compares
//! filesystem-visible behavior with the bundled upstream C littlefs
//! implementation whenever a matching oracle exists. See `tests/README.md` in
//! the repository for the current compatibility matrix.
//!
//! # Internal Layout
//!
//! The crate is intentionally split by littlefs layers:
//!
//! - `format` knows about raw on-disk numbers, tags, endian quirks, and CRCs.
//! - `commit` writes metadata block commits using the same tag/CRC machinery.
//! - `metadata` turns committed metadata logs into semantic file records.
//! - `fs` exposes read-only and mounted mutable filesystem APIs.
//! - `writer` contains explicit full-image builder/editor utilities for
//!   fixtures, host tools, and interop tests.

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

mod allocator;
mod block_device;
mod cache;
mod commit;
mod format;
mod fs;
mod metadata;
mod path;
mod types;
mod writer;

#[cfg(feature = "std")]
pub use block_device::FileBlockDevice;
pub use block_device::{BlockDevice, MemoryBlockDevice};
pub use fs::{DirHandle, FileHandle, FileOptions, FileWriter, Filesystem, FilesystemMut};
pub use types::{
    Config, DirEntry, DirectoryUsage, Error, FileType, FilesystemLimits, FilesystemOptions, FsInfo,
    InlineMax, Result, WalkEntry,
};
pub use writer::{ImageBuilder, ImageEditor};
