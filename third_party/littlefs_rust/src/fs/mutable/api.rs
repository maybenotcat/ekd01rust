impl<D: BlockDevice + 'static> FilesystemMut<D> {
    pub(super) fn mount(device: D) -> Result<Self> {
        Self::mount_with_options(device, FilesystemOptions::default())
    }

    pub(super) fn mount_with_options(device: D, options: FilesystemOptions) -> Result<Self> {
        let device = Box::new(device);
        let fs = Self::mount_device_view(&*device, options)?;
        let cache = BlockCache::new_with_cache_size(fs.cfg, fs.options().cache_size_for(fs.cfg))?;
        let needs_global_repair = !fs.global_state().is_zero();
        let used = if needs_global_repair {
            alloc::vec![true; fs.cfg.block_count]
        } else {
            Self::used_blocks_from_device_snapshot(&fs, &device)?
        };
        let allocator = BlockAllocator::from_used_with_start(
            fs.cfg,
            used,
            fs.allocation_seed() as usize,
        )?;
        let mut mounted = Self {
            device,
            cache,
            allocator,
            block_cycles: fs.options().block_cycles,
            fs,
        };
        if needs_global_repair {
            mounted.repair_global_state()?;
            mounted.refresh()?;
        }
        Ok(mounted)
    }

    fn mount_device_view(device: &D, options: FilesystemOptions) -> Result<Filesystem<'static>> {
        // `FilesystemMut` owns `device` in a `Box`, so the pointee address is
        // stable even when the outer mounted struct moves. The read-only view
        // must never escape the mounted struct; `into_device` explicitly drops
        // the view before returning the boxed device to the caller.
        let device_ref: &'static D = unsafe { &*(device as *const D) };
        Filesystem::mount_device_cached(device_ref, options, 2)
    }

    /// Reparses visible filesystem state from the owned block device.
    /// Write methods call this after durable commits so subsequent reads
    /// observe exactly what a remount would observe, without keeping a resident
    /// metadata-block image.
    pub fn refresh(&mut self) -> Result<()> {
        self.cache.sync(&mut self.device)?;
        let bad_blocks = self.allocator.bad_blocks().to_vec();
        self.fs = Self::mount_device_view(&*self.device, self.fs.options())?;
        let used = self.used_blocks_from_device()?;
        self.allocator = BlockAllocator::from_used_with_bad_blocks(
            self.fs.cfg,
            used,
            self.fs.allocation_seed() as usize,
            &bad_blocks,
        )?;
        self.cache.invalidate_all();
        Ok(())
    }

    fn refresh_after_native_write(&mut self) -> Result<()> {
        // Native write paths call `cache.sync` at the transaction boundary
        // before refreshing the parsed snapshot. Avoid issuing a second device
        // sync here: file-handle retry semantics rely on distinguishing "the
        // metadata commit reached the device" from a later explicit fsync
        // failure.
        self.fs = Self::mount_device_view(&*self.device, self.fs.options())?;
        self.cache.invalidate_all();
        Ok(())
    }

    fn rebuild_allocator_from_visible_state(&mut self) -> Result<()> {
        // Some successful native transactions make old CTZ data blocks or
        // directory pairs unreachable. Rebuilding from the freshly remounted
        // visible graph is the conservative free-space policy: allocations
        // still use the cheap mounted bitmap during a transaction, and after
        // the commit lands the bitmap stops leaking blocks that littlefs would
        // be allowed to reuse after erase.
        let bad_blocks = self.allocator.bad_blocks().to_vec();
        let used = self.used_blocks_from_device()?;
        self.allocator = BlockAllocator::from_used_with_bad_blocks(
            self.fs.cfg,
            used,
            self.fs.allocation_seed() as usize,
            &bad_blocks,
        )?;
        Ok(())
    }

    fn used_blocks_from_device(&self) -> Result<Vec<bool>> {
        Self::used_blocks_from_device_snapshot(&self.fs, &self.device)
    }

    fn used_blocks_from_device_snapshot(fs: &Filesystem<'static>, device: &D) -> Result<Vec<bool>> {
        let mut used = alloc::vec![false; fs.cfg.block_count];
        let mut seen_pairs = Vec::new();
        Self::mark_pair_tree_from_device(fs, device, &fs.root, &mut used, &mut seen_pairs)?;
        Ok(used)
    }

    fn mark_pair_tree_from_device(
        fs: &Filesystem<'static>,
        device: &D,
        pair: &MetadataPair,
        used: &mut [bool],
        seen_pairs: &mut Vec<[u32; 2]>,
    ) -> Result<()> {
        let storage_refs = Self::mark_pair_chain_from_device(fs, pair, used, seen_pairs)?;
        for storage in storage_refs {
            match storage {
                StorageRef::Directory(child) => {
                    let child = fs.read_pair(child)?;
                    Self::mark_pair_tree_from_device(fs, device, &child, used, seen_pairs)?;
                }
                StorageRef::Ctz { head, size } => {
                    Self::mark_ctz_blocks_from_device(fs.cfg, device, head, size, used)?;
                }
            }
        }
        Ok(())
    }

    fn mark_pair_chain_from_device(
        fs: &Filesystem<'static>,
        pair: &MetadataPair,
        used: &mut [bool],
        seen_pairs: &mut Vec<[u32; 2]>,
    ) -> Result<Vec<StorageRef>> {
        let mut storage_refs = Vec::new();
        let mut current = pair.clone();

        loop {
            if seen_pairs.contains(&current.pair) {
                return Err(Error::Corrupt);
            }
            seen_pairs.push(current.pair);
            Self::mark_block_used(current.pair[0], used)?;
            Self::mark_block_used(current.pair[1], used)?;
            storage_refs.extend(current.storage_refs()?);

            match current.hardtail()? {
                Some(next) if next != [LFS_NULL, LFS_NULL] => {
                    current = fs.read_pair(next)?;
                }
                _ => break,
            }
        }

        Ok(storage_refs)
    }

    fn mark_ctz_blocks_from_device(
        cfg: Config,
        device: &D,
        head: u32,
        size: u32,
        used: &mut [bool],
    ) -> Result<()> {
        let mut pos = 0u32;
        while pos < size {
            let (block, off) = Self::ctz_find_from_device(cfg, device, head, size, pos)?;
            if block == LFS_NULL {
                return Err(Error::Corrupt);
            }
            Self::mark_block_used(block, used)?;

            let block_data = Self::read_block_from_device_cfg(cfg, device, block)?;
            let off = off as usize;
            if off >= block_data.len() {
                return Err(Error::Corrupt);
            }
            let diff = core::cmp::min((size - pos) as usize, block_data.len() - off);
            pos += diff as u32;
        }
        Ok(())
    }

    fn mark_block_used(block: u32, used: &mut [bool]) -> Result<()> {
        let slot = used.get_mut(block as usize).ok_or(Error::OutOfBounds)?;
        if *slot {
            return Err(Error::Corrupt);
        }
        *slot = true;
        Ok(())
    }

    fn read_block_from_device_cfg(cfg: Config, device: &D, block: u32) -> Result<Vec<u8>> {
        if block as usize >= cfg.block_count {
            return Err(Error::OutOfBounds);
        }
        let mut out = alloc::vec![0xff; cfg.block_size];
        device.read(block, 0, &mut out)?;
        Ok(out)
    }

    fn read_block_from_device(&self, block: u32) -> Result<Vec<u8>> {
        Self::read_block_from_device_cfg(self.fs.cfg, &self.device, block)
    }

    pub(super) fn read_file_at(&self, path: &str, offset: usize, out: &mut [u8]) -> Result<usize> {
        let file = self.fs.resolve_file_no_attrs(path)?;
        if file.ty != FileType::File {
            return Err(Error::IsDir);
        }

        self.read_file_data_at(&file.data, offset, out)
    }

    pub(super) fn read_file_data_at(
        &self,
        data: &FileData,
        offset: usize,
        out: &mut [u8],
    ) -> Result<usize> {
        self.fs.read_file_data_at(data, offset, out)
    }

    fn read_ctz_from_device(&self, head: u32, size: u32) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(size as usize);
        let mut pos = 0u32;
        while pos < size {
            let (block, off) =
                Self::ctz_find_from_device(self.fs.cfg, &self.device, head, size, pos)?;
            if block == LFS_NULL {
                return Err(Error::Corrupt);
            }
            let block_data = self.read_block_from_device(block)?;
            let off = off as usize;
            if off >= block_data.len() {
                return Err(Error::Corrupt);
            }
            let diff = core::cmp::min((size - pos) as usize, block_data.len() - off);
            out.extend_from_slice(&block_data[off..off + diff]);
            pos += diff as u32;
        }
        Ok(out)
    }

    pub(super) fn ctz_blocks_from_device(&self, head: u32, size: u32) -> Result<Vec<u32>> {
        let mut blocks = Vec::new();
        let mut pos = 0usize;
        let mut index = 0usize;
        while pos < size as usize {
            let (block, off) =
                Self::ctz_find_from_device(self.fs.cfg, &self.device, head, size, pos as u32)?;
            let data_start = ctz_data_start(index)?;
            if off as usize != data_start {
                return Err(Error::Corrupt);
            }
            blocks.push(block);
            pos = pos
                .checked_add(
                    self.fs
                        .cfg
                        .block_size
                        .checked_sub(data_start)
                        .ok_or(Error::InvalidConfig)?,
                )
                .ok_or(Error::NoSpace)?;
            index = index.checked_add(1).ok_or(Error::NoSpace)?;
        }
        Ok(blocks)
    }

    fn ctz_find_from_device(
        cfg: Config,
        device: &D,
        mut head: u32,
        size: u32,
        pos: u32,
    ) -> Result<(u32, u32)> {
        if size == 0 {
            return Ok((LFS_NULL, 0));
        }

        let mut last = size - 1;
        let mut target_pos = pos;
        let mut current = Self::ctz_index_for_cfg(cfg, &mut last)?;
        let target = Self::ctz_index_for_cfg(cfg, &mut target_pos)?;

        while current > target {
            let skip = core::cmp::min(npw2(current - target + 1) - 1, ctz(current));
            let block = Self::read_block_from_device_cfg(cfg, device, head)?;
            let ptr_off = (4 * skip) as usize;
            if ptr_off + 4 > block.len() {
                return Err(Error::Corrupt);
            }
            head = le32(&block[ptr_off..ptr_off + 4])?;
            current -= 1 << skip;
        }

        Ok((head, target_pos))
    }

    fn ctz_index_for_cfg(cfg: Config, off: &mut u32) -> Result<u32> {
        let size = *off;
        let b = cfg.block_size.checked_sub(8).ok_or(Error::InvalidConfig)? as u32;
        let mut i = size / b;
        if i == 0 {
            return Ok(0);
        }

        i = (size - 4 * (popc(i - 1) + 2)) / b;
        *off = size - b * i - 4 * popc(i);
        Ok(i)
    }

    /// Returns the mounted read-only metadata view.
    ///
    /// This is useful for metadata-only inspection such as `stat`, `read_dir`,
    /// attrs, global state, and directory usage. For file payload reads through
    /// a mutable mount, prefer `FilesystemMut::read_file` or file handles:
    /// CTZ data blocks intentionally stay on the owned block device and are
    /// fetched through the view's small read cache instead of being copied into
    /// a resident metadata snapshot.
    pub fn as_filesystem(&self) -> &Filesystem<'static> {
        &self.fs
    }

    pub fn info(&self) -> &FsInfo {
        self.fs.info()
    }

    pub fn limits(&self) -> FilesystemLimits {
        self.fs.limits()
    }

    pub fn directory_usage(&self, path: &str) -> Result<DirectoryUsage> {
        self.fs.directory_usage(path)
    }

    pub fn used_blocks(&self) -> Result<Vec<bool>> {
        self.used_blocks_from_device()
    }

    pub fn read_dir(&self, path: &str) -> Result<Vec<DirEntry>> {
        self.fs.read_dir(path)
    }

    pub fn read_dir_with<F>(&self, path: &str, visitor: F) -> Result<()>
    where
        F: FnMut(DirEntry) -> Result<()>,
    {
        self.fs.read_dir_with(path, visitor)
    }

    pub fn open_dir<'fs>(&'fs self, path: &str) -> Result<DirHandle<'fs, 'static>> {
        self.fs.open_dir(path)
    }

    pub fn read_file(&self, path: &str) -> Result<Vec<u8>> {
        let file = self.fs.resolve_file_no_attrs(path)?;
        if file.ty != FileType::File {
            return Err(Error::IsDir);
        }

        match file.data {
            FileData::Inline(data) => Ok(data),
            FileData::Ctz { head, size } => self.read_ctz_from_device(head, size),
            FileData::Directory(_) => Err(Error::IsDir),
        }
    }

    pub fn read_attr(&self, path: &str, attr_type: u8) -> Result<Vec<u8>> {
        self.fs.read_attr(path, attr_type)
    }

    pub fn read_attr_into(&self, path: &str, attr_type: u8, out: &mut [u8]) -> Result<usize> {
        self.fs.read_attr_into(path, attr_type, out)
    }

    pub fn walk(&self, path: &str) -> Result<Vec<WalkEntry>> {
        self.fs.walk(path)
    }

    pub fn walk_with<F>(&self, path: &str, visitor: F) -> Result<()>
    where
        F: FnMut(WalkEntry) -> Result<()>,
    {
        self.fs.walk_with(path, visitor)
    }

    /// Creates a file through native mounted transactions only.
    ///
    /// The mounted API must not silently materialize the whole block device in
    /// RAM. If a path shape is not covered by the native writer yet, callers get
    /// `Unsupported` instead of a hidden `block_size * block_count` allocation.
    pub fn create_file(&mut self, path: &str, data: &[u8]) -> Result<()> {
        match self.fs.stat(path) {
            Ok(entry) if entry.ty == FileType::Dir => return Err(Error::IsDir),
            Ok(_) => return Err(Error::AlreadyExists),
            Err(Error::NotFound) => {}
            Err(err) => return Err(err),
        }
        if self.create_root_inline_file_native(path, data)? {
            return Ok(());
        }
        if self.create_root_ctz_file_native(path, data)? {
            return Ok(());
        }
        if self.create_child_inline_file_native(path, data)? {
            return Ok(());
        }
        if self.create_child_ctz_file_native(path, data)? {
            return Ok(());
        }

        Err(Error::Unsupported)
    }

    /// Creates or replaces a file with the provided bytes through native paths.
    pub fn write_file(&mut self, path: &str, data: &[u8]) -> Result<()> {
        let exists = match self.fs.stat(path) {
            Ok(entry) if entry.ty == FileType::File => true,
            Ok(_) => return Err(Error::IsDir),
            Err(Error::NotFound) => false,
            Err(err) => return Err(err),
        };
        if exists {
            match self.update_root_inline_file_native(path, data) {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(Error::NoSpace) => {
                    // A large inline payload can exceed the remaining metadata
                    // room in a pressured pair even when the file is still
                    // below the configured inline threshold. C littlefs can
                    // represent the same bytes as CTZ data with a tiny
                    // metadata struct, so continue to the CTZ update path
                    // before reporting exhaustion.
                }
                Err(err) => return Err(err),
            }
        }
        if exists && self.update_root_ctz_file_native(path, data)? {
            return Ok(());
        }
        if exists {
            match self.update_child_inline_file_native(path, data) {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(Error::NoSpace) => {}
                Err(err) => return Err(err),
            }
        }
        if exists {
            match self.update_child_ctz_file_native(path, data) {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(err) => return Err(err),
            }
        }

        if !exists {
            return self.create_file(path, data);
        }

        Err(Error::Unsupported)
    }

    pub fn append_file(&mut self, path: &str, data: &[u8]) -> Result<()> {
        let mut file = self.open_file(path, FileOptions::new().append(true))?;
        file.write_all(data)?;
        file.close()
    }

    /// Renames a file by copying visible bytes to the new path and removing the
    /// old path.
    ///
    /// This is not yet littlefs's atomic rename transaction. It is a deliberate
    /// mounted-API milestone that gives users same-content rename semantics on
    /// top of already C-verified create/delete operations. Power-loss tests
    /// will force this into a true atomic metadata operation later.
    pub fn rename_file(&mut self, from: &str, to: &str) -> Result<()> {
        let destination_exists = match self.fs.stat(to) {
            Ok(entry) if entry.ty == FileType::File => true,
            Ok(_) => return Err(Error::IsDir),
            Err(Error::NotFound) => false,
            Err(err) => return Err(err),
        };
        if self.rename_record_native(from, to, FileType::File)? {
            self.rebuild_allocator_from_visible_state()?;
            return Ok(());
        }
        if destination_exists {
            return Err(Error::Unsupported);
        }
        let data = self.read_file(from)?;
        self.write_file(to, &data)?;
        self.remove_file(from)
    }

    pub fn rename_dir(&mut self, from: &str, to: &str) -> Result<()> {
        let from_parts = components(from)?;
        let to_parts = components(to)?;
        if from_parts.is_empty() {
            return Err(Error::InvalidPath);
        }
        if to_parts.len() > from_parts.len()
            && to_parts
                .iter()
                .zip(from_parts.iter())
                .all(|(to, from)| to == from)
        {
            return self.rename_dir_into_own_subtree_native(from, &to_parts);
        }
        match self.fs.stat(to) {
            Ok(entry) if entry.ty == FileType::Dir => {
                if !self.fs.read_dir(to)?.is_empty() {
                    return Err(Error::NotEmpty);
                }
            }
            Ok(_) => return Err(Error::NotDir),
            Err(Error::NotFound) => {}
            Err(err) => return Err(err),
        }
        if self.rename_record_native(from, to, FileType::Dir)? {
            self.rebuild_allocator_from_visible_state()?;
            return Ok(());
        }
        Err(Error::Unsupported)
    }

    pub fn remove_file(&mut self, path: &str) -> Result<()> {
        if self.remove_root_file_native(path)? {
            return Ok(());
        }
        if self.remove_child_file_native(path)? {
            return Ok(());
        }

        Err(Error::Unsupported)
    }

    pub fn set_attr(&mut self, path: &str, attr_type: u8, data: &[u8]) -> Result<()> {
        if self.update_root_attr_native(path, attr_type, Some(data))? {
            return Ok(());
        }
        if self.update_child_attr_native(path, attr_type, Some(data))? {
            return Ok(());
        }

        Err(Error::Unsupported)
    }

    pub fn remove_attr(&mut self, path: &str, attr_type: u8) -> Result<()> {
        if self.update_root_attr_native(path, attr_type, None)? {
            return Ok(());
        }
        if self.update_child_attr_native(path, attr_type, None)? {
            return Ok(());
        }

        Err(Error::Unsupported)
    }

    pub fn create_dir(&mut self, path: &str) -> Result<()> {
        if self.create_root_dir_native(path)? {
            return Ok(());
        }
        if self.create_child_dir_native(path)? {
            return Ok(());
        }

        Err(Error::Unsupported)
    }

    pub fn remove_dir(&mut self, path: &str) -> Result<()> {
        if self.remove_root_dir_native(path)? {
            return Ok(());
        }
        if self.remove_child_dir_native(path)? {
            return Ok(());
        }

        Err(Error::Unsupported)
    }

    pub fn create_file_writer<'fs>(&'fs mut self, path: &str) -> Result<FileWriter<'fs, D>> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;
        if name.len() > self.fs.info.name_max as usize {
            return Err(Error::NameTooLong);
        }
        match self.fs.stat(path) {
            Ok(entry) if entry.ty == FileType::Dir => return Err(Error::IsDir),
            Ok(_) => return Err(Error::AlreadyExists),
            Err(Error::NotFound) => {}
            Err(err) => return Err(err),
        }
        let parent = if parents.is_empty() {
            String::from("/")
        } else {
            alloc::format!("/{}", parents.join("/"))
        };
        self.fs.resolve_dir(&parent)?;
        Ok(FileWriter {
            fs: self,
            path: path.to_string(),
            data: Vec::new(),
            pos: 0,
            stream: None,
        })
    }

    pub fn open_file<'fs>(
        &'fs mut self,
        path: &str,
        options: FileOptions,
    ) -> Result<FileHandle<'fs, D>> {
        if path.is_empty() {
            return Err(Error::InvalidPath);
        }
        let parts = components(path)?;
        let (name, _) = parts.split_last().ok_or(Error::InvalidPath)?;
        if name.len() > self.fs.info.name_max as usize {
            return Err(Error::NameTooLong);
        }
        if !options.read
            && !options.write
            && !options.append
            && !options.truncate
            && !options.create
            && !options.create_new
        {
            return Err(Error::Unsupported);
        }

        let existing_file = match self.fs.resolve_file_no_attrs(path) {
            Ok(file) if file.ty == FileType::File => Some(file),
            Ok(_) => return Err(Error::IsDir),
            Err(Error::NotFound) => None,
            Err(err) => return Err(err),
        };
        let existing = existing_file
            .as_ref()
            .map(|file| file.dir_entry().size as usize);

        if options.create_new && existing.is_some() {
            return Err(Error::AlreadyExists);
        }
        let mut len = existing.unwrap_or(0);
        let stream_target = if existing.is_some() {
            StreamingTarget::Replace
        } else {
            StreamingTarget::Create
        };
        let mut stream = None;
        let mut merge = None;
        let needs_buffer = options.write || options.append || options.truncate;
        let mut data = if existing.is_some() {
            if options.truncate {
                Vec::new()
            } else if options.append && !options.read {
                stream = self.streaming_append_state_for_existing_ctz(path)?;
                if stream.is_some() {
                    Vec::new()
                } else {
                    self.read_file(path)?
                }
            } else if options.write && !options.read {
                merge = self.partial_overwrite_state_for_existing_ctz(path)?;
                if merge.is_some() {
                    Vec::new()
                } else {
                    self.read_file(path)?
                }
            } else if needs_buffer {
                self.read_file(path)?
            } else {
                Vec::new()
            }
        } else if options.create || options.create_new {
            Vec::new()
        } else {
            return Err(Error::NotFound);
        };
        let stream_read = options.read && !needs_buffer && existing.is_some();
        let stream_source = stream_read.then(|| {
            existing_file
                .as_ref()
                .expect("existing file was resolved for read handle")
                .data
                .clone()
        });

        let mut dirty = existing.is_none() && (options.create || options.create_new);
        if options.truncate {
            data.clear();
            len = 0;
            dirty = true;
        }
        let pos = if options.append { len } else { 0 };
        Ok(FileHandle {
            fs: self,
            path: path.to_string(),
            data,
            pos,
            len,
            stream_read,
            stream_source,
            stream_target,
            stream,
            merge,
            readable: options.read,
            writable: options.write || options.append,
            dirty,
        })
    }

    pub fn sync(&mut self) -> Result<()> {
        self.cache.sync(&mut self.device)
    }

    /// Configures the mounted metadata relocation threshold.
    ///
    /// littlefs treats `block_cycles` as a wear-leveling hint: once a metadata
    /// pair has been compacted enough times, the pair should move to freshly
    /// allocated blocks and its parent should be updated to point at the new
    /// pair. `None` disables this mounted-writer relocation policy. The default
    /// mirrors the C fixture (`128`), while tests can lower the value to force
    /// relocation without hundreds of commits.
    pub fn set_block_cycles(&mut self, block_cycles: Option<u32>) {
        self.block_cycles = block_cycles;
    }

    pub fn into_device(self) -> D {
        let Self {
            fs,
            device,
            cache: _,
            allocator: _,
            block_cycles: _,
        } = self;
        drop(fs);
        *device
    }
}

impl<D: BlockDevice + Clone + 'static> Clone for FilesystemMut<D> {
    fn clone(&self) -> Self {
        let device = Box::new(self.device.as_ref().clone());
        let fs = Self::mount_device_view(&*device, self.fs.options())
            .expect("clone mounted filesystem view");
        Self {
            fs,
            device,
            cache: self.cache.clone(),
            allocator: self.allocator.clone(),
            block_cycles: self.block_cycles,
        }
    }
}
