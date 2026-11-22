use super::*;

impl<'a> ImageStorage<'a> {
    fn full_len_is_available(&self, cfg: Config) -> bool {
        let Some(expected) = cfg.block_size.checked_mul(cfg.block_count) else {
            return false;
        };
        match self {
            ImageStorage::Borrowed(image) => image.len() >= expected,
            ImageStorage::Owned(image) => image.len() >= expected,
            ImageStorage::Device { device, .. } => {
                let device_cfg = device.config();
                device_cfg.block_size == cfg.block_size && device_cfg.block_count >= cfg.block_count
            }
        }
    }

    fn read(&self, cfg: Config, block: u32, off: usize, out: &mut [u8]) -> Result<()> {
        if block as usize >= cfg.block_count {
            return Err(Error::OutOfBounds);
        }
        if off.checked_add(out.len()).ok_or(Error::OutOfBounds)? > cfg.block_size {
            return Err(Error::OutOfBounds);
        }

        match self {
            ImageStorage::Borrowed(image) => {
                let start = block as usize * cfg.block_size + off;
                let end = start + out.len();
                out.copy_from_slice(image.get(start..end).ok_or(Error::OutOfBounds)?);
                Ok(())
            }
            ImageStorage::Owned(image) => {
                let start = block as usize * cfg.block_size + off;
                let end = start + out.len();
                out.copy_from_slice(image.get(start..end).ok_or(Error::OutOfBounds)?);
                Ok(())
            }
            ImageStorage::Device { device, cache } => cache.read(*device, cfg, block, off, out),
        }
    }

    fn read_block(&self, cfg: Config, block: u32, out: &mut [u8]) -> Result<()> {
        if out.len() != cfg.block_size {
            return Err(Error::InvalidConfig);
        }
        self.read(cfg, block, 0, out)
    }
}

impl<'a> Filesystem<'a> {
    /// Mounts an existing littlefs image by reading the root metadata pair
    /// `{0, 1}` and validating the superblock entry.
    pub fn mount(image: &'a [u8], cfg: Config) -> Result<Self> {
        Self::mount_with_options(image, cfg, FilesystemOptions::default())
    }

    /// Mounts an existing image with explicit littlefs-style options.
    pub fn mount_with_options(
        image: &'a [u8],
        cfg: Config,
        options: FilesystemOptions,
    ) -> Result<Self> {
        Self::mount_storage(ImageStorage::Borrowed(image), cfg, options)
    }

    /// Mounts a read-only view by borrowing a block device.
    ///
    /// Unlike the fixture-oriented `mount(&[u8])` path, this does not copy the
    /// complete device into an owned `Vec`. Metadata and CTZ data blocks are
    /// fetched through `BlockDevice::read` as user operations need them. The
    /// returned filesystem therefore cannot outlive the device reference.
    pub fn mount_device<D: BlockDevice + 'a>(device: &'a D) -> Result<Self> {
        Self::mount_device_with_options(device, FilesystemOptions::default())
    }

    /// Mounts a read-only block-device view with explicit cache and limit
    /// options. The limit fields are used like C littlefs' mount-time maximums:
    /// an image whose superblock advertises larger limits is rejected.
    pub fn mount_device_with_options<D: BlockDevice + 'a>(
        device: &'a D,
        options: FilesystemOptions,
    ) -> Result<Self> {
        Self::mount_device_cached(device, options, 4)
    }

    pub(crate) fn mount_device_cached<D: BlockDevice + 'a>(
        device: &'a D,
        options: FilesystemOptions,
        cache_slots: usize,
    ) -> Result<Self> {
        let cfg = device.config();
        let options = options.validate(cfg)?;
        Self::mount_storage(
            ImageStorage::Device {
                device,
                cache: Rc::new(ReadBlockCache::new(
                    options.cache_size_for(cfg),
                    cache_slots,
                )),
            },
            cfg,
            options,
        )
    }

    fn mount_storage(
        image: ImageStorage<'a>,
        cfg: Config,
        options: FilesystemOptions,
    ) -> Result<Self> {
        let options = options.validate(cfg)?;
        if cfg.block_size < 16 || cfg.block_count < 2 {
            return Err(Error::InvalidConfig);
        }
        if !image.full_len_is_available(cfg) {
            return Err(Error::OutOfBounds);
        }

        let root = Self::read_pair_from_storage(&image, cfg, [0, 1])?;

        // The superblock is not a special block. It is a normal metadata entry
        // rooted in the first metadata pair, and the magic string is stored in
        // the name tag with id 0.
        let name = root.find(Tag::new(LFS_TYPE_SUPERBLOCK, 0, 8))?;
        if name.data != b"littlefs" {
            return Err(Error::Corrupt);
        }

        let sb = root.find(Tag::new(LFS_TYPE_INLINESTRUCT, 0, 24))?;
        if sb.data.len() != 24 {
            return Err(Error::Corrupt);
        }
        let info = FsInfo {
            disk_version: le32(&sb.data[0..4])?,
            block_size: le32(&sb.data[4..8])?,
            block_count: le32(&sb.data[8..12])?,
            name_max: le32(&sb.data[12..16])?,
            file_max: le32(&sb.data[16..20])?,
            attr_max: le32(&sb.data[20..24])?,
        };

        if info.disk_version != SUPPORTED_DISK_VERSION {
            return Err(Error::Unsupported);
        }
        if info.block_size as usize != cfg.block_size {
            return Err(Error::InvalidConfig);
        }
        if info.block_count as usize > cfg.block_count {
            return Err(Error::InvalidConfig);
        }
        if info.name_max > options.name_max
            || info.file_max > options.file_max
            || info.attr_max > options.attr_max
        {
            return Err(Error::InvalidConfig);
        }

        let global_state = Self::collect_global_state(&image, cfg, &root)?;
        let allocation_seed = Self::collect_allocation_seed(&image, cfg, &root)?;

        Ok(Self {
            image,
            cfg,
            root,
            info,
            options,
            global_state,
            allocation_seed,
        })
    }

    pub fn info(&self) -> &FsInfo {
        &self.info
    }

    pub fn options(&self) -> FilesystemOptions {
        self.options
    }

    pub fn limits(&self) -> FilesystemLimits {
        FilesystemLimits {
            block_size: self.info.block_size,
            block_count: self.info.block_count,
            name_max: self.info.name_max,
            file_max: self.info.file_max,
            attr_max: self.info.attr_max,
        }
    }

    pub(super) fn global_state(&self) -> GlobalState {
        self.global_state
    }

    pub(super) fn allocation_seed(&self) -> u32 {
        self.allocation_seed
    }

    pub(super) fn inline_threshold(&self) -> usize {
        self.options.inline_threshold(self.cfg, self.info.attr_max)
    }

    pub(super) fn read_pair(&self, pair: [u32; 2]) -> Result<MetadataPair> {
        Self::read_pair_from_storage(&self.image, self.cfg, pair)
    }

    fn read_pair_from_storage(
        image: &ImageStorage<'_>,
        cfg: Config,
        pair: [u32; 2],
    ) -> Result<MetadataPair> {
        MetadataPair::read_from(cfg, pair, |block, out| image.read_block(cfg, block, out))
    }

    pub fn directory_usage(&self, path: &str) -> Result<DirectoryUsage> {
        let pair = self.resolve_dir(path)?;
        self.directory_usage_from_pair(&pair)
    }

    pub fn root_entries(&self) -> Result<Vec<DirEntry>> {
        self.read_dir("/")
    }

    pub fn read_dir(&self, path: &str) -> Result<Vec<DirEntry>> {
        let pair = self.resolve_dir(path)?;
        self.entries_in_pair(&pair)
    }

    /// Visits directory entries without allocating the public result `Vec`.
    ///
    /// The parser still folds one metadata pair at a time internally, but
    /// callers that only need to stream names or count entries no longer have
    /// to materialize the complete directory listing. `read_dir` is intentionally
    /// kept as the convenient collecting wrapper.
    pub fn read_dir_with<F>(&self, path: &str, mut visitor: F) -> Result<()>
    where
        F: FnMut(DirEntry) -> Result<()>,
    {
        let pair = self.resolve_dir(path)?;
        self.for_each_file_in_pair_chain(&pair, |file| visitor(file.dir_entry()))
    }

    pub fn open_dir<'fs>(&'fs self, path: &str) -> Result<DirHandle<'fs, 'a>> {
        let pair = self.resolve_dir(path)?;
        Ok(DirHandle {
            fs: self,
            head: pair.pair,
            pos: 0,
        })
    }

    pub fn stat(&self, path: &str) -> Result<DirEntry> {
        // littlefs reports the root as a directory even though it is not named
        // by a normal directory entry.
        if path.is_empty() || path == "/" {
            return Ok(DirEntry {
                name: alloc::string::String::new(),
                ty: FileType::Dir,
                size: 0,
            });
        }

        self.resolve_dir_entry(path)
    }

    pub fn read_file(&self, path: &str) -> Result<Vec<u8>> {
        let file = self.resolve_file_no_attrs(path)?;
        if file.ty != FileType::File {
            return Err(Error::IsDir);
        }

        match file.data {
            FileData::Inline(data) => Ok(data),
            FileData::Ctz { head, size } => self.read_ctz(head, size),
            FileData::Directory(_) => Err(Error::IsDir),
        }
    }

    /// Reads a file into a caller-provided buffer and returns the byte count.
    ///
    /// This is the embedded-facing full-file read shape. It uses the normal
    /// metadata/path machinery internally, but file contents do not require
    /// allocating a result `Vec`. If the buffer is too small, no partial-read
    /// contract is promised; callers get `NoSpace` and can retry with the size
    /// reported by `stat`.
    pub fn read_file_into(&self, path: &str, out: &mut [u8]) -> Result<usize> {
        let file = self.resolve_file_no_attrs(path)?;
        if file.ty != FileType::File {
            return Err(Error::IsDir);
        }

        let size = file.dir_entry().size as usize;
        if out.len() < size {
            return Err(Error::NoSpace);
        }

        match file.data {
            FileData::Inline(data) => {
                out[..data.len()].copy_from_slice(&data);
                Ok(data.len())
            }
            FileData::Ctz { head, size } => self.read_ctz_into(head, size, out),
            FileData::Directory(_) => Err(Error::IsDir),
        }
    }

    /// Reads a range from a file without materializing the whole file.
    ///
    /// This is the read-only counterpart to the mutable file-handle streaming
    /// path. It follows littlefs' visible read semantics: offsets at or beyond
    /// EOF return zero, and short tail reads return only the remaining bytes.
    pub fn read_file_at(&self, path: &str, offset: usize, out: &mut [u8]) -> Result<usize> {
        let file = self.resolve_file_no_attrs(path)?;
        if file.ty != FileType::File {
            return Err(Error::IsDir);
        }

        let size = file.dir_entry().size as usize;
        if offset >= size || out.is_empty() {
            return Ok(0);
        }
        let n = core::cmp::min(out.len(), size - offset);
        match file.data {
            FileData::Inline(data) => {
                out[..n].copy_from_slice(&data[offset..offset + n]);
                Ok(n)
            }
            FileData::Ctz { head, size } => self.read_ctz_range(head, size, offset, &mut out[..n]),
            FileData::Directory(_) => Err(Error::IsDir),
        }
    }

    pub(super) fn read_file_data_at(
        &self,
        data: &FileData,
        offset: usize,
        out: &mut [u8],
    ) -> Result<usize> {
        let size = match data {
            FileData::Inline(data) => data.len(),
            FileData::Ctz { size, .. } => *size as usize,
            FileData::Directory(_) => return Err(Error::IsDir),
        };
        if offset >= size || out.is_empty() {
            return Ok(0);
        }
        let n = core::cmp::min(out.len(), size - offset);
        match data {
            FileData::Inline(data) => {
                out[..n].copy_from_slice(&data[offset..offset + n]);
                Ok(n)
            }
            FileData::Ctz { head, size } => {
                self.read_ctz_range(*head, *size, offset, &mut out[..n])
            }
            FileData::Directory(_) => Err(Error::IsDir),
        }
    }

    pub fn read_attr(&self, path: &str, attr_type: u8) -> Result<Vec<u8>> {
        self.resolve_attr(path, attr_type)?.ok_or(Error::NotFound)
    }

    /// Reads a user attribute into caller-owned storage.
    ///
    /// This mirrors `read_file_into`: a missing attr returns `NotFound`, and a
    /// too-small output buffer returns `NoSpace` without copying a truncated
    /// value. The return value is the number of bytes written.
    pub fn read_attr_into(&self, path: &str, attr_type: u8, out: &mut [u8]) -> Result<usize> {
        self.resolve_attr_into(path, attr_type, out)?
            .ok_or(Error::NotFound)
    }

    pub fn walk(&self, path: &str) -> Result<Vec<WalkEntry>> {
        let base = normalize_dir_path(path)?;
        let mut out = Vec::new();
        self.walk_with(&base, |entry| {
            out.push(entry);
            Ok(())
        })?;
        Ok(out)
    }

    /// Recursively visits entries without allocating a full-tree result.
    ///
    /// Each directory is still sorted with a short-lived local vector so the
    /// callback order matches `walk`. Avoiding a single full recursive result is
    /// enough for embedded callers that stream a manifest or stop early after a
    /// match.
    pub fn walk_with<F>(&self, path: &str, mut visitor: F) -> Result<()>
    where
        F: FnMut(WalkEntry) -> Result<()>,
    {
        let base = normalize_dir_path(path)?;
        self.walk_dir_with(&base, &mut visitor)
    }

    /// Scans the mounted image and returns a bitmap of blocks reachable from
    /// metadata pairs, hardtail chains, directory structs, and CTZ files.
    ///
    /// This is intentionally conservative: both sides of every metadata pair
    /// are marked used even if only one side currently contains the newest
    /// valid commit. That matches littlefs allocation semantics and gives the
    /// future mount-and-mutate writer a safe starting point.
    pub fn used_blocks(&self) -> Result<Vec<bool>> {
        let mut used = alloc::vec![false; self.cfg.block_count];
        let mut seen_pairs = Vec::new();
        self.mark_pair_tree(&self.root, &mut used, &mut seen_pairs)?;
        Ok(used)
    }

    fn walk_dir_with<F>(&self, path: &str, visitor: &mut F) -> Result<()>
    where
        F: FnMut(WalkEntry) -> Result<()>,
    {
        let mut entries = self.read_dir(path)?;
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        for entry in entries {
            let child_path = join_path(path, &entry.name);
            visitor(WalkEntry {
                path: child_path.clone(),
                entry: entry.clone(),
            })?;
            if entry.ty == FileType::Dir {
                self.walk_dir_with(&child_path, visitor)?;
            }
        }

        Ok(())
    }

    pub(super) fn resolve_dir(&self, path: &str) -> Result<MetadataPair> {
        // Resolve one path component at a time with targeted metadata lookup.
        // Directory listing still materializes a public Vec for callers, but
        // `stat("/a/b")` should not allocate every entry in `/` just to find
        // `a`, then allocate every entry in `/a` just to find `b`.
        if path.is_empty() || path == "/" {
            return Ok(self.root.clone());
        }

        let mut pair = self.root.clone();
        for component in components(path)? {
            let file = self.find_file_in_pair_chain_no_attrs(&pair, component)?;
            match file.data {
                FileData::Directory(child) if file.ty == FileType::Dir => {
                    pair = self.read_pair(child)?;
                }
                _ => return Err(Error::NotDir),
            }
        }
        Ok(pair)
    }

    fn resolve_dir_entry(&self, path: &str) -> Result<DirEntry> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;

        let mut pair = self.root.clone();
        for component in parents {
            let file = self.find_file_in_pair_chain_no_attrs(&pair, component)?;
            match file.data {
                FileData::Directory(child) if file.ty == FileType::Dir => {
                    pair = self.read_pair(child)?;
                }
                _ => return Err(Error::NotDir),
            }
        }

        self.find_dir_entry_in_pair_chain(&pair, name)
    }

    pub(super) fn resolve_file_no_attrs(&self, path: &str) -> Result<FileRecord> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;

        let mut pair = self.root.clone();
        for component in parents {
            let file = self.find_file_in_pair_chain_no_attrs(&pair, component)?;
            match file.data {
                FileData::Directory(child) if file.ty == FileType::Dir => {
                    pair = self.read_pair(child)?;
                }
                _ => return Err(Error::NotDir),
            }
        }

        self.find_file_in_pair_chain_no_attrs(&pair, name)
    }

    fn resolve_attr(&self, path: &str, attr_type: u8) -> Result<Option<Vec<u8>>> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;

        let mut pair = self.root.clone();
        for component in parents {
            let file = self.find_file_in_pair_chain_no_attrs(&pair, component)?;
            match file.data {
                FileData::Directory(child) if file.ty == FileType::Dir => {
                    pair = self.read_pair(child)?;
                }
                _ => return Err(Error::NotDir),
            }
        }

        self.find_attr_in_pair_chain(&pair, name, attr_type)
    }

    fn resolve_attr_into(
        &self,
        path: &str,
        attr_type: u8,
        out: &mut [u8],
    ) -> Result<Option<usize>> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;

        let mut pair = self.root.clone();
        for component in parents {
            let file = self.find_file_in_pair_chain_no_attrs(&pair, component)?;
            match file.data {
                FileData::Directory(child) if file.ty == FileType::Dir => {
                    pair = self.read_pair(child)?;
                }
                _ => return Err(Error::NotDir),
            }
        }

        self.find_attr_in_pair_chain_into(&pair, name, attr_type, out)
    }

    fn entries_in_pair(&self, pair: &MetadataPair) -> Result<Vec<DirEntry>> {
        let mut entries = Vec::new();
        self.for_each_file_in_pair_chain(pair, |file| {
            entries.push(file.dir_entry());
            Ok(())
        })?;
        Ok(entries)
    }

    pub(super) fn dir_entry_at(&self, head: [u32; 2], index: usize) -> Result<Option<DirEntry>> {
        let mut current = if head == self.root.pair {
            self.root.clone()
        } else {
            self.read_pair(head)?
        };
        let mut seen = Vec::<[u32; 2]>::new();
        let mut remaining = index;

        loop {
            if seen.contains(&current.pair) {
                return Err(Error::Corrupt);
            }
            seen.push(current.pair);

            let mut found = None;
            let mut local_index = 0usize;
            current.fold_dir(|_id, file| {
                if found.is_none() && local_index == remaining {
                    found = Some(file.dir_entry());
                }
                local_index += 1;
                Ok(())
            })?;
            if found.is_some() {
                return Ok(found);
            }
            remaining = remaining.saturating_sub(local_index);

            match current.hardtail()? {
                Some(next) if next != [LFS_NULL, LFS_NULL] => {
                    current = self.read_pair(next)?;
                }
                _ => return Ok(None),
            }
        }
    }

    pub(super) fn files_in_pair_chain(&self, pair: &MetadataPair) -> Result<Vec<FileRecord>> {
        let mut files = Vec::new();
        self.for_each_file_in_pair_chain(pair, |file| {
            files.push(file);
            Ok(())
        })?;
        Ok(files)
    }

    pub(super) fn for_each_file_in_pair_chain<F>(
        &self,
        pair: &MetadataPair,
        mut visitor: F,
    ) -> Result<()>
    where
        F: FnMut(FileRecord) -> Result<()>,
    {
        let mut current = pair.clone();
        let mut seen = Vec::<[u32; 2]>::new();

        loop {
            // Hardtail loops should never exist on a valid filesystem, but bad
            // images must not turn traversal into an infinite loop.
            if seen.contains(&current.pair) {
                return Err(Error::Corrupt);
            }
            seen.push(current.pair);
            current.fold_dir(|_id, file| visitor(file))?;

            match current.hardtail()? {
                Some(next) if next != [LFS_NULL, LFS_NULL] => {
                    current = self.read_pair(next)?;
                }
                _ => break,
            }
        }

        Ok(())
    }

    fn find_file_in_pair_chain_no_attrs(
        &self,
        pair: &MetadataPair,
        name: &str,
    ) -> Result<FileRecord> {
        let mut current = pair.clone();
        let mut seen = Vec::<[u32; 2]>::new();

        loop {
            if seen.contains(&current.pair) {
                return Err(Error::Corrupt);
            }
            seen.push(current.pair);

            if let Some(file) = current.find_name_no_attrs(name)? {
                return Ok(file);
            }

            match current.hardtail()? {
                Some(next) if next != [LFS_NULL, LFS_NULL] => {
                    current = self.read_pair(next)?;
                }
                _ => return Err(Error::NotFound),
            }
        }
    }

    fn find_dir_entry_in_pair_chain(&self, pair: &MetadataPair, name: &str) -> Result<DirEntry> {
        let mut current = pair.clone();
        let mut seen = Vec::<[u32; 2]>::new();

        loop {
            if seen.contains(&current.pair) {
                return Err(Error::Corrupt);
            }
            seen.push(current.pair);

            if let Some(entry) = current.find_dir_entry(name)? {
                return Ok(entry);
            }

            match current.hardtail()? {
                Some(next) if next != [LFS_NULL, LFS_NULL] => {
                    current = self.read_pair(next)?;
                }
                _ => return Err(Error::NotFound),
            }
        }
    }

    fn find_attr_in_pair_chain(
        &self,
        pair: &MetadataPair,
        name: &str,
        attr_type: u8,
    ) -> Result<Option<Vec<u8>>> {
        let mut current = pair.clone();
        let mut seen = Vec::<[u32; 2]>::new();

        loop {
            if seen.contains(&current.pair) {
                return Err(Error::Corrupt);
            }
            seen.push(current.pair);

            if let Some(attr) = current.find_attr(name, attr_type)? {
                return Ok(Some(attr));
            }

            match current.hardtail()? {
                Some(next) if next != [LFS_NULL, LFS_NULL] => {
                    current = self.read_pair(next)?;
                }
                _ => return Ok(None),
            }
        }
    }

    fn find_attr_in_pair_chain_into(
        &self,
        pair: &MetadataPair,
        name: &str,
        attr_type: u8,
        out: &mut [u8],
    ) -> Result<Option<usize>> {
        let mut current = pair.clone();
        let mut seen = Vec::<[u32; 2]>::new();

        loop {
            if seen.contains(&current.pair) {
                return Err(Error::Corrupt);
            }
            seen.push(current.pair);

            if let Some(len) = current.copy_attr_into(name, attr_type, out)? {
                return Ok(Some(len));
            }

            match current.hardtail()? {
                Some(next) if next != [LFS_NULL, LFS_NULL] => {
                    current = self.read_pair(next)?;
                }
                _ => return Ok(None),
            }
        }
    }

    fn directory_usage_from_pair(&self, pair: &MetadataPair) -> Result<DirectoryUsage> {
        let mut entry_count = 0usize;
        let mut metadata_pair_count = 0usize;
        let mut current = pair.clone();
        let mut seen = Vec::<[u32; 2]>::new();
        let mut is_split = false;
        loop {
            if seen.contains(&current.pair) {
                return Err(Error::Corrupt);
            }
            seen.push(current.pair);
            metadata_pair_count += 1;
            entry_count += current.file_count()?;
            match current.hardtail()? {
                Some(next) if next != [LFS_NULL, LFS_NULL] => {
                    is_split = true;
                    current = self.read_pair(next)?;
                }
                _ => {
                    return Ok(DirectoryUsage {
                        entry_count,
                        metadata_pair_count,
                        is_split,
                        append_bytes_used: current.state.off,
                        append_bytes_remaining: self
                            .cfg
                            .block_size
                            .saturating_sub(current.state.off),
                    });
                }
            }
        }
    }

    fn mark_pair_tree(
        &self,
        pair: &MetadataPair,
        used: &mut [bool],
        seen_pairs: &mut Vec<[u32; 2]>,
    ) -> Result<()> {
        let files = self.mark_pair_chain(pair, used, seen_pairs)?;
        for file in files {
            match file.data {
                FileData::Directory(child) if file.ty == FileType::Dir => {
                    let child = self.read_pair(child)?;
                    self.mark_pair_tree(&child, used, seen_pairs)?;
                }
                FileData::Ctz { head, size } if file.ty == FileType::File => {
                    self.mark_ctz_blocks(head, size, used)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn mark_pair_chain(
        &self,
        pair: &MetadataPair,
        used: &mut [bool],
        seen_pairs: &mut Vec<[u32; 2]>,
    ) -> Result<Vec<FileRecord>> {
        let mut files = Vec::new();
        let mut current = pair.clone();

        loop {
            if seen_pairs.contains(&current.pair) {
                return Err(Error::Corrupt);
            }
            seen_pairs.push(current.pair);
            self.mark_block(current.pair[0], used)?;
            self.mark_block(current.pair[1], used)?;
            files.extend(current.files()?);

            match current.hardtail()? {
                Some(next) if next != [LFS_NULL, LFS_NULL] => {
                    current = self.read_pair(next)?;
                }
                _ => break,
            }
        }

        Ok(files)
    }

    fn mark_ctz_blocks(&self, head: u32, size: u32, used: &mut [bool]) -> Result<()> {
        let mut pos = 0u32;
        while pos < size {
            let (block, off) = self.ctz_find(head, size, pos)?;
            if block == LFS_NULL {
                return Err(Error::Corrupt);
            }
            self.mark_block(block, used)?;

            let off = off as usize;
            if off >= self.cfg.block_size {
                return Err(Error::Corrupt);
            }
            let diff = core::cmp::min((size - pos) as usize, self.cfg.block_size - off);
            pos += diff as u32;
        }
        Ok(())
    }

    fn mark_block(&self, block: u32, used: &mut [bool]) -> Result<()> {
        let slot = used.get_mut(block as usize).ok_or(Error::OutOfBounds)?;
        if *slot {
            return Err(Error::Corrupt);
        }
        *slot = true;
        Ok(())
    }

    fn collect_global_state(
        image: &ImageStorage<'_>,
        cfg: Config,
        root: &MetadataPair,
    ) -> Result<GlobalState> {
        if root.tail()?.is_some() {
            return Self::collect_global_state_thread(image, cfg, root);
        }
        let mut global_state = GlobalState::default();
        let mut seen_pairs = Vec::new();
        Self::collect_global_state_pair_tree(image, cfg, root, &mut seen_pairs, &mut global_state)?;
        Ok(global_state)
    }

    fn collect_allocation_seed(
        image: &ImageStorage<'_>,
        cfg: Config,
        root: &MetadataPair,
    ) -> Result<u32> {
        if root.tail()?.is_some() {
            return Self::collect_allocation_seed_thread(image, cfg, root);
        }
        let mut seed = 0;
        let mut seen_pairs = Vec::new();
        Self::collect_allocation_seed_pair_tree(image, cfg, root, &mut seen_pairs, &mut seed)?;
        Ok(seed)
    }

    fn collect_allocation_seed_thread(
        image: &ImageStorage<'_>,
        cfg: Config,
        root: &MetadataPair,
    ) -> Result<u32> {
        let mut seed = 0;
        let mut current = root.clone();
        let mut seen = Vec::<[u32; 2]>::new();
        loop {
            if seen.contains(&current.pair) {
                break;
            }
            seen.push(current.pair);
            current.fold_commit_crcs_into_seed(&mut seed)?;

            let Some(tail) = current.tail()? else {
                break;
            };
            if tail.pair == [LFS_NULL, LFS_NULL] {
                break;
            }
            current = Self::read_pair_from_storage(image, cfg, tail.pair)?;
        }
        Ok(seed)
    }

    fn collect_allocation_seed_pair_tree(
        image: &ImageStorage<'_>,
        cfg: Config,
        pair: &MetadataPair,
        seen_pairs: &mut Vec<[u32; 2]>,
        seed: &mut u32,
    ) -> Result<()> {
        if seen_pairs.contains(&pair.pair) {
            return Err(Error::Corrupt);
        }
        seen_pairs.push(pair.pair);
        pair.fold_commit_crcs_into_seed(seed)?;
        for file in pair.files()? {
            if let FileData::Directory(child) = file.data
                && file.ty == FileType::Dir
            {
                let child = Self::read_pair_from_storage(image, cfg, child)?;
                Self::collect_allocation_seed_pair_tree(image, cfg, &child, seen_pairs, seed)?;
            }
        }
        Ok(())
    }

    fn collect_global_state_thread(
        image: &ImageStorage<'_>,
        cfg: Config,
        root: &MetadataPair,
    ) -> Result<GlobalState> {
        let mut global_state = GlobalState::default();
        let mut current = root.clone();
        let mut seen = Vec::<[u32; 2]>::new();
        loop {
            if seen.contains(&current.pair) {
                // Keep read-only mount tolerant of a malformed hardtail cycle
                // so the operation that actually follows the directory chain
                // reports the corruption. This mirrors the older visible-tree
                // global-state fold and keeps corruption errors localized to
                // the user-visible traversal that needs the chain.
                break;
            }
            seen.push(current.pair);
            global_state.xor(current.global_state_delta()?);

            let Some(tail) = current.tail()? else {
                break;
            };
            if tail.pair == [LFS_NULL, LFS_NULL] {
                break;
            }
            current = Self::read_pair_from_storage(image, cfg, tail.pair)?;
        }
        Ok(global_state)
    }

    fn collect_global_state_pair_tree(
        image: &ImageStorage<'_>,
        cfg: Config,
        pair: &MetadataPair,
        seen_pairs: &mut Vec<[u32; 2]>,
        global_state: &mut GlobalState,
    ) -> Result<()> {
        let files =
            Self::collect_global_state_pair_chain(image, cfg, pair, seen_pairs, global_state)?;
        for file in files {
            if let FileData::Directory(child) = file.data
                && file.ty == FileType::Dir
            {
                let child = Self::read_pair_from_storage(image, cfg, child)?;
                Self::collect_global_state_pair_tree(image, cfg, &child, seen_pairs, global_state)?;
            }
        }
        Ok(())
    }

    fn collect_global_state_pair_chain(
        image: &ImageStorage<'_>,
        cfg: Config,
        pair: &MetadataPair,
        seen_pairs: &mut Vec<[u32; 2]>,
        global_state: &mut GlobalState,
    ) -> Result<Vec<FileRecord>> {
        let mut files = Vec::new();
        let mut current = pair.clone();
        loop {
            if seen_pairs.contains(&current.pair) {
                // Cross-parent directory rename currently has a final-state
                // path that can briefly make the same child pair reachable
                // through both old and new parents while the old entry is
                // being unlinked. Global-state aggregation should not reject
                // that mount-time intermediate or XOR the same pair twice.
                return Ok(Vec::new());
            }
            seen_pairs.push(current.pair);
            global_state.xor(current.global_state_delta()?);
            files.extend(current.files()?);

            match current.hardtail()? {
                Some(next) if next != [LFS_NULL, LFS_NULL] => {
                    current = Self::read_pair_from_storage(image, cfg, next)?;
                }
                _ => break,
            }
        }
        Ok(files)
    }

    fn read_ctz(&self, head: u32, size: u32) -> Result<Vec<u8>> {
        let mut out = alloc::vec![0; size as usize];
        self.read_ctz_range(head, size, 0, &mut out)?;
        Ok(out)
    }

    fn read_ctz_into(&self, head: u32, size: u32, out: &mut [u8]) -> Result<usize> {
        if out.len() < size as usize {
            return Err(Error::NoSpace);
        }

        self.read_ctz_range(head, size, 0, &mut out[..size as usize])
    }

    /// Reads a logical byte range from a CTZ file.
    ///
    /// CTZ lookup can jump directly to the block containing `offset`, so range
    /// reads should not scan every earlier data block. This is especially
    /// important for file handles that issue many small random reads.
    fn read_ctz_range(&self, head: u32, size: u32, offset: usize, out: &mut [u8]) -> Result<usize> {
        if offset > size as usize {
            return Ok(0);
        }
        let end = core::cmp::min(size as usize, offset.saturating_add(out.len()));
        let mut pos = u32::try_from(offset).map_err(|_| Error::OutOfBounds)?;
        let mut dst = 0usize;
        while pos < size && (pos as usize) < end {
            let (block, off) = self.ctz_find(head, size, pos)?;
            if block == LFS_NULL {
                return Err(Error::Corrupt);
            }

            let off = off as usize;
            if off >= self.cfg.block_size {
                return Err(Error::Corrupt);
            }
            let diff = core::cmp::min((size - pos) as usize, self.cfg.block_size - off);
            let logical_start = pos as usize;
            let logical_end = logical_start + diff;
            if logical_end > offset {
                let copy_start = core::cmp::max(logical_start, offset);
                let copy_end = core::cmp::min(logical_end, end);
                let src_start = off + (copy_start - logical_start);
                let len = copy_end - copy_start;
                self.image
                    .read(self.cfg, block, src_start, &mut out[dst..dst + len])?;
                dst += len;
            }
            pos += diff as u32;
        }

        Ok(dst)
    }

    fn ctz_find(&self, mut head: u32, size: u32, pos: u32) -> Result<(u32, u32)> {
        if size == 0 {
            return Ok((LFS_NULL, 0));
        }

        let mut last = size - 1;
        let mut target_pos = pos;
        let mut current = self.ctz_index(&mut last)?;
        let target = self.ctz_index(&mut target_pos)?;

        while current > target {
            // Each CTZ block contains skip pointers at the front. The pointer
            // chosen here mirrors `lfs_ctz_find`: jump back by the largest
            // usable power-of-two distance toward the target block index.
            let skip = core::cmp::min(npw2(current - target + 1) - 1, ctz(current));
            let ptr_off = (4 * skip) as usize;
            if ptr_off + 4 > self.cfg.block_size {
                return Err(Error::Corrupt);
            }
            let mut ptr = [0u8; 4];
            self.image.read(self.cfg, head, ptr_off, &mut ptr)?;
            head = le32(&ptr)?;
            current -= 1 << skip;
        }

        Ok((head, target_pos))
    }

    fn ctz_index(&self, off: &mut u32) -> Result<u32> {
        // littlefs reserves space at the front of CTZ blocks for skip pointers.
        // This helper maps a file byte offset to the logical CTZ block index
        // and rewrites `off` to the offset within that physical block.
        let size = *off;
        let b = self
            .cfg
            .block_size
            .checked_sub(8)
            .ok_or(Error::InvalidConfig)? as u32;
        let mut i = size / b;
        if i == 0 {
            return Ok(0);
        }

        i = (size - 4 * (popc(i - 1) + 2)) / b;
        *off = size - b * i - 4 * popc(i);
        Ok(i)
    }
}

impl Filesystem<'static> {

    /// Initialize only the root metadata pair. Free data blocks are erased when
    /// allocated, matching embedded littlefs first-format behavior. The caller
    /// must own the entire device; no previous filesystem is preserved.
    pub fn format_device_lazy_with_options<D: BlockDevice>(
        device: &mut D,
        options: FilesystemOptions,
    ) -> Result<()> {
        let cfg = device.config();
        let options = options.validate(cfg)?;
        if cfg.block_count < 2 { return Err(Error::InvalidConfig); }
        let root = ImageBuilder::empty_root_block_with_options(cfg, options)?;
        let mut cache = BlockCache::new_with_cache_size(cfg, options.cache_size_for(cfg))?;
        cache.erase(device, 0)?;
        cache.erase(device, 1)?;
        cache.prog(device, 0, 0, &root)?;
        cache.sync(device)
    }

    /// Formats a block device as an empty littlefs image.
    ///
    /// The formatted bytes are programmed into the caller's real block device
    /// with erase/program/sync operations, so tests exercise the public device
    /// boundary instead of only comparing an in-memory `Vec`.
    pub fn format_device<D: BlockDevice>(device: &mut D) -> Result<()> {
        Self::format_device_with_options(device, FilesystemOptions::default())
    }

    /// Formats a block device using explicit littlefs-style options.
    pub fn format_device_with_options<D: BlockDevice>(
        device: &mut D,
        options: FilesystemOptions,
    ) -> Result<()> {
        let cfg = device.config();
        let options = options.validate(cfg)?;
        let root = ImageBuilder::empty_root_block_with_options(cfg, options)?;
        let mut cache = BlockCache::new_with_cache_size(cfg, options.cache_size_for(cfg))?;
        for block in 0..cfg.block_count {
            cache.erase(device, block as u32)?;
        }
        cache.prog(device, 0, 0, &root)?;
        cache.sync(device)
    }

    /// Mounts a block device for mutation.
    ///
    /// Write methods will be added to `FilesystemMut` in small C-verified
    /// milestones. Today this establishes the API ownership model and ensures
    /// callers can format, mount, inspect, sync, and recover the underlying
    /// device without dropping down to image bytes.
    pub fn mount_device_mut<D: BlockDevice + 'static>(device: D) -> Result<FilesystemMut<D>> {
        FilesystemMut::mount(device)
    }

    /// Mounts a block device for mutation with explicit littlefs-style options.
    pub fn mount_device_mut_with_options<D: BlockDevice + 'static>(
        device: D,
        options: FilesystemOptions,
    ) -> Result<FilesystemMut<D>> {
        FilesystemMut::mount_with_options(device, options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::LFS_TYPE_MOVESTATE;

    fn append_movestate(image: &mut [u8], cfg: Config, pair: &MetadataPair, delta: GlobalState) {
        let start = pair.active_block as usize * cfg.block_size;
        let end = start + cfg.block_size;
        let block = image.get_mut(start..end).expect("metadata block slice");
        let mut payload = Vec::new();
        payload.extend_from_slice(&delta.tag.to_le_bytes());
        payload.extend_from_slice(&delta.pair[0].to_le_bytes());
        payload.extend_from_slice(&delta.pair[1].to_le_bytes());
        let entry = CommitEntry::new(Tag::new(LFS_TYPE_MOVESTATE, 0x3ff, 12), &payload);
        let mut writer =
            MetadataCommitWriter::append(block, 16, pair.state).expect("append movestate commit");
        writer
            .write_entries(&[entry])
            .expect("write movestate entry");
        writer.finish().expect("finish movestate commit");
    }

    #[test]
    fn mount_aggregates_global_state_from_visible_pair_tree() {
        let cfg = Config {
            block_size: 512,
            block_count: 32,
        };
        let mut builder = ImageBuilder::new(cfg).expect("builder");
        builder.create_dir("/docs").expect("create child directory");
        builder
            .add_inline_file("/docs/a.txt", b"a")
            .expect("create child file");
        let mut image = builder.build().expect("build image");
        let fs = Filesystem::mount(&image, cfg).expect("mount initial image");
        let root = fs.root.clone();
        let docs = fs.resolve_dir("/docs").expect("resolve child dir");

        let root_delta = GlobalState {
            tag: Tag::new(LFS_TYPE_DELETE, 7, 0).0,
            pair: [0x10, 0x11],
        };
        let docs_delta = GlobalState {
            tag: Tag::new(LFS_TYPE_DELETE, 3, 0).0,
            pair: [0x05, 0x15],
        };
        append_movestate(&mut image, cfg, &root, root_delta);
        append_movestate(&mut image, cfg, &docs, docs_delta);

        let fs = Filesystem::mount(&image, cfg).expect("mount image with global state");
        assert_eq!(
            fs.global_state(),
            GlobalState {
                tag: root_delta.tag ^ docs_delta.tag,
                pair: [
                    root_delta.pair[0] ^ docs_delta.pair[0],
                    root_delta.pair[1] ^ docs_delta.pair[1],
                ],
            }
        );
    }
}
