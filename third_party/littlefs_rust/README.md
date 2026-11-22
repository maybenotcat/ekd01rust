# littlefs-rust

`littlefs-rust` is a pure Rust implementation of the littlefs v2 on-disk
format, published on crates.io as `littlefs2-rust` because the shorter package
name is already occupied. It can format, mount, read, and mutate littlefs
images through a synchronous block-device trait, while keeping core library
code compatible with `no_std + alloc`.

Inspired by littlefs. The implementation is independent Rust code, and
interoperability with upstream littlefs remains a design constraint.

The project is built around real interoperability instead of format sketches:
the default test suite writes real images, remounts them, and compares visible
filesystem behavior with the bundled upstream C littlefs implementation in
`littlefs_c/` wherever a C oracle exists.

## Status

The crate currently supports the common mounted filesystem surface needed by
embedded-style applications:

| Area | Supported surface |
| --- | --- |
| Device model | `BlockDevice`, `MemoryBlockDevice`, and default-`std` `FileBlockDevice` |
| Lifecycle | `format_device`, read-only `mount_device`, owned mutable `mount_device_mut` |
| Read APIs | `stat`, `read_dir`, `open_dir`, `walk`, `read_file`, `read_file_into`, `read_file_at`, attrs |
| Write APIs | create, overwrite, append, remove, mkdir, rmdir, rename, user attrs |
| File handles | open flags, read/write/append/truncate, seek, flush, sync, close |
| Large files | CTZ file reads and writes, streaming create and append paths |
| Directories | nested directories, hardtail split chains, split-aware lookup/update/delete/rename/attrs |
| Durability | data-before-metadata write ordering, selected power-loss and fault-injection tests |
| Resource shape | mounted block-device paths avoid resident whole-image ownership |

This is not a byte-for-byte rewrite of every internal C state machine. Some
low-level policy details, especially allocator checkpointing, complete
wear-leveling fidelity, and rare relocation or split-compaction interleavings,
remain documented compatibility work. The public filesystem behavior is already
covered by broad Rust/C black-box tests.

## Installation

```toml
[dependencies]
littlefs-rust = { package = "littlefs2-rust", version = "0.1" }
```

For a `no_std` build, disable default features. The crate still requires
`alloc` from the target environment.

```toml
[dependencies]
littlefs-rust = { package = "littlefs2-rust", version = "0.1", default-features = false }
```

## Quick Start

```rust
use littlefs_rust::{Config, FileBlockDevice, FileOptions, Filesystem};

fn main() -> littlefs_rust::Result<()> {
    let cfg = Config {
        block_size: 512,
        block_count: 4096,
    };

    let path = std::env::temp_dir().join("littlefs-rust-example.img");
    let mut device = FileBlockDevice::create_erased(&path, cfg)?;
    Filesystem::format_device(&mut device)?;

    let mut fs = Filesystem::mount_device_mut(device)?;
    fs.create_dir("/logs")?;

    let mut file = fs.open_file(
        "/logs/current.txt",
        FileOptions::new()
            .create(true)
            .write(true)
            .append(true),
    )?;
    file.write_all(b"boot ok\n")?;
    file.sync()?;
    file.close()?;

    fs.append_file("/logs/current.txt", b"network ok\n")?;
    fs.sync()?;

    let device = fs.into_device();
    let ro = Filesystem::mount_device(&device)?;
    assert_eq!(ro.read_file("/logs/current.txt")?, b"boot ok\nnetwork ok\n");

    let _ = std::fs::remove_file(path);
    Ok(())
}
```

## Configuration

`Config` describes physical geometry:

```rust
use littlefs_rust::Config;

let cfg = Config {
    block_size: 512,
    block_count: 4096,
};
```

`FilesystemOptions` mirrors the main non-callback fields of C
`struct lfs_config` and is accepted by the `*_with_options` format and mount
APIs. It covers read, program, and cache sizes, lookahead size validation,
`block_cycles`, name/file/attr limits, `metadata_max`, and inline-file policy.

```rust
use littlefs_rust::{FilesystemOptions, InlineMax};

let options = FilesystemOptions {
    cache_size: 128,
    prog_size: 16,
    read_size: 16,
    inline_max: InlineMax::Limit(64),
    ..FilesystemOptions::default()
};
```

See [docs/configuration.md](docs/configuration.md) for validation rules,
feature flags, and block-device expectations.

## Block Device Contract

The block-device trait uses littlefs vocabulary:

- `read(block, off, out)` reads bytes inside one logical block.
- `prog(block, off, data)` programs bytes inside one block. NOR-style backends
  should only change bits from `1` to `0` until erase.
- `erase(block)` resets a whole block to the erased value.
- `sync()` flushes durable backend state when the backend needs it.

The bundled `MemoryBlockDevice` and `FileBlockDevice` are real block devices
used by integration tests. They are not filesystem behavior mocks.

## Documentation

- [docs/configuration.md](docs/configuration.md): public configuration and
  device contract.
- [docs/api_surface.md](docs/api_surface.md): API audit and release surface.
- [docs/architecture.md](docs/architecture.md): implementation architecture.
- [docs/sync_semantics.md](docs/sync_semantics.md): mounted sync and file sync
  semantics.
- [docs/atomic_rename.md](docs/atomic_rename.md): rename/orphan design notes.
- [tests/README.md](tests/README.md): test layout and C oracle discipline.
- [tools/blackbox_eval](tools/blackbox_eval): manual resource/performance
  evaluation harness.

## Testing

Run the default compatibility suite:

```sh
cargo fmt -- --check
cargo check --no-default-features
cargo test
```

The suite includes black-box integration tests that exercise real image bytes
and compare with upstream C littlefs. Performance and memory evaluation is kept
out of default `cargo test`; see
[tools/blackbox_eval/README.md](tools/blackbox_eval/README.md).

## Known Gaps

The implementation is intended to be useful today, but the remaining gaps are
tracked openly:

- allocator policy still uses an in-memory visible-block bitmap plus a
  lookahead scan window rather than a fully C-like persistent checkpoint
  traversal;
- bad-block and relocation retry behavior covers the important public paths,
  but not every rare split/relocation/orphan interleaving from C littlefs;
- split-chain support preserves public lookup and mutation behavior, while not
  reproducing every adjacent-pair balancing choice in `lfs_dir_splittingcompact`;
- `read/write` handles with arbitrary-position extension may still choose a
  simpler buffered path to preserve read-your-writes semantics;
- the resource profile is now in the tens of KiB for common mounted workloads,
  but the C implementation remains smaller on very tight targets.

## Release Notes

See [CHANGELOG.md](CHANGELOG.md).

## License

This crate is distributed under the MIT License. The upstream littlefs C
reference is used as a Git submodule for compatibility tests and keeps its own
upstream license.
