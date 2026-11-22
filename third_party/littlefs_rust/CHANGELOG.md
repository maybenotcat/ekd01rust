# Changelog

All notable public changes are tracked here. Dates use UTC.

## 0.1.1 - 2026-05-18

Initial release of the current pure Rust mounted filesystem implementation,
published as `littlefs2-rust`.

### Added

- Pure Rust littlefs v2 parser and writer core.
- `BlockDevice` trait with `MemoryBlockDevice` and default-`std`
  `FileBlockDevice` implementations.
- Device formatting through `Filesystem::format_device` and
  `Filesystem::format_device_with_options`.
- Read-only mounted access through `Filesystem::mount_device`.
- Owned mutable mounted access through `Filesystem::mount_device_mut`.
- Common filesystem operations: stat, directory listing, recursive walk,
  file reads, caller-buffered reads, user attributes, file create/write/append,
  remove, mkdir, rmdir, rename, open-file handles, flush, sync, and close.
- CTZ file support for large-file reads and writes, including streaming create
  and append paths.
- Split-chain directory support for common create, update, delete, attr,
  rename, and child-directory operations.
- `FilesystemOptions` and `InlineMax` for littlefs-style public configuration.
- `no_std + alloc` core build with a default `std` feature for host helpers.
- C littlefs differential and black-box tests over real image bytes.
- Manual resource/performance evaluation harness under `tools/blackbox_eval`.

### Known Gaps

- Allocator policy is compatible at the public behavior level, but still uses
  an in-memory visible-block bitmap plus scan window rather than C's complete
  checkpoint traversal and wear-leveling policy.
- Relocation, split compaction, orphan cleanup, and bad-block retry have broad
  coverage for supported operations, but are not byte-for-byte copies of every
  rare C internal state-machine path.
- The C implementation is still smaller on very tight RAM targets.

### Notes

- The crate is MIT licensed.
- The implementation is inspired by littlefs; the upstream C littlefs project
  is kept as a fixed submodule in the repository for compatibility testing.
