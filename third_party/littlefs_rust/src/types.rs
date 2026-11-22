use alloc::string::String;

/// Errors returned by the parser and mounted write API.
///
/// The variants intentionally mirror the littlefs errno classes once those
/// differences become visible through the public filesystem API. Internal
/// implementation limits still use `Unsupported` until a C-observable behavior
/// demands a narrower error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidConfig,
    OutOfBounds,
    Corrupt,
    NotFound,
    AlreadyExists,
    NotDir,
    IsDir,
    NotEmpty,
    BadFileDescriptor,
    InvalidPath,
    NameTooLong,
    FileTooLarge,
    Unsupported,
    NoSpace,
    Utf8,
    Io,
}

pub type Result<T> = core::result::Result<T, Error>;

/// Minimal mount-time geometry required to read an existing image.
///
/// The superblock is still checked after parsing; this config tells the parser
/// how to slice the raw byte image into logical blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    pub block_size: usize,
    pub block_count: usize,
}

impl Config {
    pub const DEFAULT_CACHE_SIZE: usize = 256;

    #[cfg(any(feature = "std", test))]
    pub(crate) fn cache_size(self) -> usize {
        core::cmp::min(self.block_size, Self::DEFAULT_CACHE_SIZE)
    }
}

/// Policy knobs that mirror the non-callback fields of C littlefs'
/// `struct lfs_config`.
///
/// `Config` deliberately remains only the physical geometry. That keeps the
/// old API and all block devices simple, while this type carries the settings
/// that affect formatting, mounting caches, metadata commit padding, write
/// thresholds, and the limits recorded in the superblock. The default values
/// match the C fixture geometry used by the test oracle, with cache/prog sizes
/// kept at the current low-memory Rust defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilesystemOptions {
    pub read_size: usize,
    pub prog_size: usize,
    pub cache_size: usize,
    pub lookahead_size: usize,
    pub block_cycles: Option<u32>,
    pub name_max: u32,
    pub file_max: u32,
    pub attr_max: u32,
    pub metadata_max: Option<usize>,
    pub inline_max: InlineMax,
}

/// Inline-file limit policy.
///
/// C littlefs gives `inline_max = 0` the special meaning "derive the largest
/// safe inline threshold", while `(lfs_size_t)-1` disables inline storage. A
/// Rust `usize` alone cannot represent both sentinels cleanly, so the public
/// API uses an explicit enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineMax {
    /// Derive C's default `min(cache_size, attr_max, metadata_limit / 8)`.
    Default,
    /// Disable non-empty inline file data, matching C's `(lfs_size_t)-1`.
    Disabled,
    /// Use an explicit upper bound after validating it against C's constraints.
    Limit(usize),
}

impl Default for FilesystemOptions {
    fn default() -> Self {
        Self {
            read_size: 16,
            prog_size: 16,
            cache_size: Config::DEFAULT_CACHE_SIZE,
            // The C fixture currently uses a 64-byte lookahead. Our allocator
            // still keeps an in-memory bitmap, but accepting the field now
            // makes the public API shape match littlefs and gives the future
            // checkpoint allocator a stable home.
            lookahead_size: 64,
            block_cycles: Some(128),
            name_max: 255,
            file_max: 2_147_483_647,
            attr_max: 1_022,
            metadata_max: None,
            inline_max: InlineMax::Default,
        }
    }
}

impl FilesystemOptions {
    pub(crate) fn validate(self, cfg: Config) -> Result<Self> {
        if cfg.block_size == 0
            || cfg.block_count < 2
            || self.read_size == 0
            || self.prog_size == 0
            || self.cache_size == 0
            || self.lookahead_size == 0
        {
            return Err(Error::InvalidConfig);
        }

        // These are the same divisibility relationships C checks before it
        // allocates caches or performs alignment arithmetic.
        let cache_size = self.cache_size_for(cfg);
        if cache_size % self.read_size != 0
            || cache_size % self.prog_size != 0
            || cfg.block_size % cache_size != 0
        {
            return Err(Error::InvalidConfig);
        }
        if !self.prog_size.is_power_of_two() {
            return Err(Error::InvalidConfig);
        }
        if self.lookahead_size % 8 != 0 {
            return Err(Error::InvalidConfig);
        }
        if matches!(self.block_cycles, Some(0)) {
            return Err(Error::InvalidConfig);
        }
        if self.name_max == 0
            || self.name_max > 1_022
            || self.file_max == 0
            || self.file_max > 2_147_483_647
            || self.attr_max == 0
            || self.attr_max > 1_022
        {
            return Err(Error::InvalidConfig);
        }

        if let Some(metadata_max) = self.metadata_max {
            if metadata_max == 0
                || metadata_max > cfg.block_size
                || metadata_max % self.read_size != 0
                || metadata_max % self.prog_size != 0
                || cfg.block_size % metadata_max != 0
            {
                return Err(Error::InvalidConfig);
            }
        }

        if let InlineMax::Limit(limit) = self.inline_max {
            let metadata_limit = self.metadata_max.unwrap_or(cfg.block_size);
            if limit > cache_size || limit > self.attr_max as usize || limit > metadata_limit / 8 {
                return Err(Error::InvalidConfig);
            }
        }

        Ok(self)
    }

    pub(crate) fn cache_size_for(self, cfg: Config) -> usize {
        core::cmp::min(self.cache_size, cfg.block_size)
    }

    pub(crate) fn inline_threshold(self, cfg: Config, attr_max: u32) -> usize {
        match self.inline_max {
            InlineMax::Default => {
                let metadata_limit = self.metadata_max.unwrap_or(cfg.block_size);
                core::cmp::min(
                    self.cache_size_for(cfg),
                    core::cmp::min(attr_max as usize, metadata_limit / 8),
                )
            }
            InlineMax::Disabled => 0,
            InlineMax::Limit(limit) => core::cmp::min(limit, attr_max as usize),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsInfo {
    pub disk_version: u32,
    pub block_size: u32,
    pub block_count: u32,
    pub name_max: u32,
    pub file_max: u32,
    pub attr_max: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesystemLimits {
    pub block_size: u32,
    pub block_count: u32,
    pub name_max: u32,
    pub file_max: u32,
    pub attr_max: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryUsage {
    pub entry_count: usize,
    pub metadata_pair_count: usize,
    pub is_split: bool,
    pub append_bytes_used: usize,
    pub append_bytes_remaining: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    File,
    Dir,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub ty: FileType,
    pub size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkEntry {
    pub path: String,
    pub entry: DirEntry,
}
