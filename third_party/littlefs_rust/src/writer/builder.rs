use super::*;

impl ImageBuilder {
    /// Creates a builder for a fresh littlefs image.
    ///
    /// The writer currently emits one root metadata pair and no data blocks, so
    /// it only accepts geometries large enough to contain the root pair. More
    /// constraints will move into a richer writer config once CTZ files and a
    /// real allocator appear.
    pub fn new(cfg: Config) -> Result<Self> {
        Self::new_with_options(cfg, FilesystemOptions::default())
    }

    /// Creates a fresh-image builder with explicit littlefs-style options.
    ///
    /// The options decide which limits are recorded in the superblock and which
    /// program boundary the metadata commit writer pads to. They intentionally
    /// share validation with mounted formatting so a hand-built test image does
    /// not quietly use a shape the top-level formatter would reject.
    pub fn new_with_options(cfg: Config, options: FilesystemOptions) -> Result<Self> {
        let options = options.validate(cfg)?;
        if cfg.block_size < 64 || cfg.block_count < 2 {
            return Err(Error::InvalidConfig);
        }
        Ok(Self {
            cfg,
            options,
            entries: BTreeMap::new(),
            visible_entries: BTreeMap::new(),
            update_commits: Vec::new(),
            allocator: FreshAllocator::new(cfg),
        })
    }

    pub(crate) fn empty_root_block_with_options(
        cfg: Config,
        options: FilesystemOptions,
    ) -> Result<Vec<u8>> {
        let builder = Self::new_with_options(cfg, options)?;
        let root = RootCommit::from_builder(&builder)?;
        let mut block = vec![0xff; cfg.block_size];
        root.write_into(&mut block, cfg, builder.options.prog_size)?;
        Ok(block)
    }

    /// Adds or replaces an inline file.
    ///
    /// Root files and files inside already-created directories are supported.
    /// The writer resolves the parent directory first, then inserts the file in
    /// that directory's sorted id space.
    pub fn add_inline_file(&mut self, path: &str, data: &[u8]) -> Result<&mut Self> {
        if !self.update_commits.is_empty() {
            return Err(Error::Unsupported);
        }
        let parts = components(path)?;
        let (name, parents) = split_parent(&parts)?;
        if !parents.is_empty() {
            return self.add_directory_inline_file(parents, name, data);
        }
        if name.len() > self.options.name_max as usize
            || data.len()
                > self
                    .options
                    .inline_threshold(self.cfg, self.options.attr_max)
        {
            return Err(Error::Unsupported);
        }
        if matches!(self.entries.get(name), Some(RootEntry::Dir(_))) {
            return Err(Error::Unsupported);
        }

        self.entries.insert(
            name.to_string(),
            RootEntry::File(InlineFile {
                storage: FileStorage::Inline(data.to_vec()),
                attrs: BTreeMap::new(),
            }),
        );
        self.visible_entries
            .insert(name.to_string(), RootKind::File);
        Ok(self)
    }

    /// Adds a CTZ file.
    ///
    /// CTZ data blocks are allocated monotonically in the fresh image and are
    /// linked with littlefs skip pointers. Root files and files inside existing
    /// directories share the same CTZ data-block writer.
    pub fn add_ctz_file(&mut self, path: &str, data: &[u8]) -> Result<&mut Self> {
        if !self.update_commits.is_empty() {
            return Err(Error::Unsupported);
        }
        let parts = components(path)?;
        let (name, parents) = split_parent(&parts)?;
        if name.len() > self.options.name_max as usize {
            return Err(Error::Unsupported);
        }
        if !parents.is_empty() {
            self.directory(parents)?;
            let blocks = self.allocator.alloc_ctz_blocks(data.len())?;
            let parent = self.directory_mut(parents)?;
            return parent.add_ctz_file(name, data, blocks).map(|_| self);
        }
        if matches!(self.entries.get(name), Some(RootEntry::Dir(_))) {
            return Err(Error::Unsupported);
        }
        let blocks = self.allocator.alloc_ctz_blocks(data.len())?;
        self.entries.insert(
            name.to_string(),
            RootEntry::File(InlineFile {
                storage: FileStorage::Ctz(CtzFile::new(data, blocks)),
                attrs: BTreeMap::new(),
            }),
        );
        self.visible_entries
            .insert(name.to_string(), RootKind::File);
        Ok(self)
    }

    /// Adds an empty directory before append-style updates are emitted.
    ///
    /// Each directory receives a fresh metadata pair from the builder's
    /// monotonic allocator.
    pub fn create_dir(&mut self, path: &str) -> Result<&mut Self> {
        if !self.update_commits.is_empty() {
            return Err(Error::Unsupported);
        }
        let parts = components(path)?;
        let (name, parents) = split_parent(&parts)?;
        if name.len() > self.options.name_max as usize {
            return Err(Error::Unsupported);
        }
        if !parents.is_empty() {
            self.directory(parents)?;
            let pair = self.allocator.alloc_pair()?;
            let parent = self.directory_mut(parents)?;
            return parent.create_dir(name, pair).map(|_| self);
        }
        if self.entries.contains_key(name) {
            return Err(Error::Unsupported);
        }
        let pair = self.allocator.alloc_pair()?;
        self.entries.insert(
            name.to_string(),
            RootEntry::Dir(Directory::new(pair, self.cfg, self.options)),
        );
        self.visible_entries.insert(name.to_string(), RootKind::Dir);
        Ok(self)
    }

    /// Sets a user attribute on an already-added root-level inline file.
    ///
    /// littlefs encodes user attributes as metadata tags with type
    /// `LFS_TYPE_USERATTR + attr_type`. Keeping this method tied to files that
    /// already exist avoids accidentally emitting orphan attributes that C would
    /// ignore or reject in later operations.
    pub fn set_attr(&mut self, path: &str, attr_type: u8, data: &[u8]) -> Result<&mut Self> {
        if !self.update_commits.is_empty() {
            return Err(Error::Unsupported);
        }
        let parts = components(path)?;
        let (name, parents) = split_parent(&parts)?;
        if data.len() > self.options.attr_max as usize {
            return Err(Error::Unsupported);
        }
        if !parents.is_empty() {
            let parent = self.directory_mut(parents)?;
            return parent.set_attr(name, attr_type, data).map(|_| self);
        }

        let entry = self.entries.get_mut(name).ok_or(Error::NotFound)?;
        let RootEntry::File(file) = entry else {
            return Err(Error::Unsupported);
        };
        file.attrs.insert(attr_type, data.to_vec());
        Ok(self)
    }

    /// Appends a later root metadata commit that overwrites an inline file.
    ///
    /// The original create/name tags remain in the first commit, while this
    /// method writes a later struct tag for the same id. littlefs's normal
    /// supersede rules make the later inline payload visible.
    pub fn update_inline_file(&mut self, path: &str, data: &[u8]) -> Result<&mut Self> {
        let parts = components(path)?;
        let (name, parents) = split_parent(&parts)?;
        if !parents.is_empty() {
            let parent = self.directory_mut(parents)?;
            return parent.update_inline_file(name, data).map(|_| self);
        }
        if data.len()
            > self
                .options
                .inline_threshold(self.cfg, self.options.attr_max)
        {
            return Err(Error::Unsupported);
        }
        let id = root_entry_id(&self.visible_entries, name)?;
        match self.visible_entries.get(name) {
            Some(RootKind::File) => {}
            Some(RootKind::Dir) => return Err(Error::Unsupported),
            None => return Err(Error::NotFound),
        }

        self.push_root_storage_update(id, FileStorage::Inline(data.to_vec()));
        Ok(self)
    }

    /// Updates an existing file, selecting inline or CTZ storage by payload size.
    ///
    /// A later littlefs struct tag supersedes the previous one even when the
    /// struct type changes. That lets this builder model inline-to-CTZ and
    /// CTZ-to-inline conversion as ordinary append commits. Old CTZ blocks are
    /// left unreachable until the allocator learns real free-space tracking.
    pub fn update_file(&mut self, path: &str, data: &[u8]) -> Result<&mut Self> {
        if data.len()
            <= self
                .options
                .inline_threshold(self.cfg, self.options.attr_max)
        {
            return self.update_inline_file(path, data);
        }

        let parts = components(path)?;
        let (name, parents) = split_parent(&parts)?;
        if !parents.is_empty() {
            let parent = self.directory(parents)?;
            child_file_id(&parent.visible_entries, name)?;
            let blocks = self.allocator.alloc_ctz_blocks(data.len())?;
            let parent = self.directory_mut(parents)?;
            return parent
                .update_storage(name, FileStorage::Ctz(CtzFile::new(data, blocks)))
                .map(|_| self);
        }

        let id = root_entry_id(&self.visible_entries, name)?;
        match self.visible_entries.get(name) {
            Some(RootKind::File) => {}
            Some(RootKind::Dir) => return Err(Error::Unsupported),
            None => return Err(Error::NotFound),
        }

        let blocks = self.allocator.alloc_ctz_blocks(data.len())?;
        self.push_root_storage_update(id, FileStorage::Ctz(CtzFile::new(data, blocks)));
        Ok(self)
    }

    fn push_root_storage_update(&mut self, id: u16, storage: FileStorage) {
        self.update_commits.push(RootUpdateCommit {
            id,
            storage: Some(storage),
            attrs: BTreeMap::new(),
            delete_file: false,
        });
    }

    /// Appends a later root metadata commit that overwrites a user attribute.
    ///
    /// Empty attributes are represented as normal zero-length userattr tags.
    /// Deletion uses a different tag shape and is handled by `delete_attr`.
    pub fn update_attr(&mut self, path: &str, attr_type: u8, data: &[u8]) -> Result<&mut Self> {
        let parts = components(path)?;
        let (name, parents) = split_parent(&parts)?;
        if data.len() > self.options.attr_max as usize {
            return Err(Error::Unsupported);
        }
        if !parents.is_empty() {
            let parent = self.directory_mut(parents)?;
            return parent.update_attr(name, attr_type, data).map(|_| self);
        }
        let id = root_entry_id(&self.visible_entries, name)?;
        match self.visible_entries.get(name) {
            Some(RootKind::File) => {}
            Some(RootKind::Dir) => return Err(Error::Unsupported),
            None => return Err(Error::NotFound),
        }

        let mut attrs = BTreeMap::new();
        attrs.insert(attr_type, Some(data.to_vec()));
        self.update_commits.push(RootUpdateCommit {
            id,
            storage: None,
            attrs,
            delete_file: false,
        });
        Ok(self)
    }

    /// Appends a later root metadata commit that deletes a user attribute.
    ///
    /// C's `lfs_removeattr` is just a normal metadata commit with a userattr tag
    /// whose size field is `0x3ff`. In littlefs tag terminology that means
    /// "delete the latest matching tag" rather than "write a 1023-byte value".
    pub fn delete_attr(&mut self, path: &str, attr_type: u8) -> Result<&mut Self> {
        let parts = components(path)?;
        let (name, parents) = split_parent(&parts)?;
        if !parents.is_empty() {
            let parent = self.directory_mut(parents)?;
            return parent.delete_attr(name, attr_type).map(|_| self);
        }
        let id = root_entry_id(&self.visible_entries, name)?;
        match self.visible_entries.get(name) {
            Some(RootKind::File) => {}
            Some(RootKind::Dir) => return Err(Error::Unsupported),
            None => return Err(Error::NotFound),
        }

        let mut attrs = BTreeMap::new();
        attrs.insert(attr_type, None);
        self.update_commits.push(RootUpdateCommit {
            id,
            storage: None,
            attrs,
            delete_file: false,
        });
        Ok(self)
    }

    /// Appends a later root metadata commit that deletes a root inline file.
    ///
    /// File deletion is a splice operation in littlefs. The `LFS_TYPE_DELETE`
    /// tag removes the current id and shifts later ids down by one, so the
    /// builder updates `visible_entries` immediately. Later update/delete calls
    /// therefore compute ids against the same state that C will see.
    pub fn delete_file(&mut self, path: &str) -> Result<&mut Self> {
        let parts = components(path)?;
        let (name, parents) = split_parent(&parts)?;
        if !parents.is_empty() {
            let parent = self.directory_mut(parents)?;
            return parent.delete_file(name).map(|_| self);
        }
        let id = root_entry_id(&self.visible_entries, name)?;
        if self.visible_entries.remove(name).is_none() {
            return Err(Error::NotFound);
        }

        self.update_commits.push(RootUpdateCommit {
            id,
            storage: None,
            attrs: BTreeMap::new(),
            delete_file: true,
        });
        Ok(self)
    }

    /// Appends a parent-directory commit that removes an empty directory.
    ///
    /// littlefs uses the same splice/delete tag for files and directories in a
    /// parent directory. The safety rule lives above the tag writer: only a
    /// directory whose final visible child map is empty may be unlinked here.
    /// Its old metadata pair remains allocated but unreachable until a real
    /// free-space tracker exists.
    pub fn delete_dir(&mut self, path: &str) -> Result<&mut Self> {
        let parts = components(path)?;
        let (name, parents) = split_parent(&parts)?;
        if !parents.is_empty() {
            let parent = self.directory_mut(parents)?;
            return parent.delete_dir(name).map(|_| self);
        }

        let id = root_entry_id(&self.visible_entries, name)?;
        match self.visible_entries.get(name) {
            Some(RootKind::Dir) => {}
            Some(RootKind::File) => return Err(Error::Unsupported),
            None => return Err(Error::NotFound),
        }
        let RootEntry::Dir(dir) = self.entries.get(name).ok_or(Error::Corrupt)? else {
            return Err(Error::Unsupported);
        };
        if !dir.is_empty_for_delete() {
            return Err(Error::Unsupported);
        }

        self.visible_entries.remove(name);
        self.update_commits.push(RootUpdateCommit {
            id,
            storage: None,
            attrs: BTreeMap::new(),
            delete_file: true,
        });
        Ok(self)
    }

    /// Builds the erased flash image and writes the root metadata commit.
    ///
    /// Blocks start as `0xff`, the erased NOR-flash state. We then program only
    /// block 0 with a complete metadata commit. A valid single side of the root
    /// pair is sufficient for both littlefs and our parser; block 1 remains an
    /// erased power-loss-style alternate copy.
    pub fn build(&self) -> Result<Vec<u8>> {
        let image_len = self
            .cfg
            .block_size
            .checked_mul(self.cfg.block_count)
            .ok_or(Error::InvalidConfig)?;
        let mut image = vec![0xff; image_len];
        for entry in self.entries.values() {
            match entry {
                RootEntry::Dir(dir) => dir.write_empty_pair(&mut image, self.cfg)?,
                RootEntry::File(file) => {
                    file.storage.write_blocks(&mut image, self.cfg)?;
                }
            }
        }
        for update in &self.update_commits {
            update.write_blocks(&mut image, self.cfg)?;
        }
        if let Err(err) = self.write_root_log(&mut image) {
            if err != Error::NoSpace {
                return Err(err);
            }

            // If the append log does not fit, compact the final visible root
            // state to the other side of the root pair. This is not full
            // littlefs wear-leveling yet, but it proves the essential recovery
            // shape: C chooses the newer valid revision and sees only compacted
            // state.
            let compacted = self.compacted_root_entries()?;
            let root = RootCommit::from_entries(self.cfg, self.options, &compacted)?;
            root.write_into_rev(
                &mut image[self.cfg.block_size..2 * self.cfg.block_size],
                self.cfg,
                self.options.prog_size,
                2,
            )?;
        }
        Ok(image)
    }

    fn write_root_log(&self, image: &mut [u8]) -> Result<()> {
        let root = RootCommit::from_builder(self)?;
        let mut state = root.write_into(
            &mut image[0..self.cfg.block_size],
            self.cfg,
            self.options.prog_size,
        )?;
        for update in &self.update_commits {
            state = update.write_into(
                &mut image[0..self.cfg.block_size],
                self.cfg,
                self.options.prog_size,
                state,
            )?;
        }
        Ok(())
    }

    fn compacted_root_entries(&self) -> Result<BTreeMap<String, RootEntry>> {
        let mut entries = self.entries.clone();
        for update in &self.update_commits {
            let key = root_key_for_id(&entries, update.id)?.to_string();
            if update.delete_file {
                entries.remove(&key);
                continue;
            }

            let entry = entries.get_mut(&key).ok_or(Error::Corrupt)?;
            let RootEntry::File(file) = entry else {
                return Err(Error::Unsupported);
            };
            if let Some(storage) = &update.storage {
                file.storage = storage.clone();
            }
            for (attr_type, attr) in &update.attrs {
                match attr {
                    Some(data) => {
                        file.attrs.insert(*attr_type, data.clone());
                    }
                    None => {
                        file.attrs.remove(attr_type);
                    }
                }
            }
        }
        Ok(entries)
    }
}

impl ImageBuilder {
    fn add_directory_inline_file(
        &mut self,
        parents: &[&str],
        name: &str,
        data: &[u8],
    ) -> Result<&mut Self> {
        let parent = self.directory_mut(parents)?;
        parent.add_inline_file(name, data)?;
        Ok(self)
    }

    fn directory(&self, path: &[&str]) -> Result<&Directory> {
        let (name, rest) = path.split_first().ok_or(Error::Unsupported)?;
        let entry = self.entries.get(*name).ok_or(Error::NotFound)?;
        let RootEntry::Dir(dir) = entry else {
            return Err(Error::Unsupported);
        };
        if rest.is_empty() {
            Ok(dir)
        } else {
            dir.directory(rest)
        }
    }

    fn directory_mut(&mut self, path: &[&str]) -> Result<&mut Directory> {
        let (name, rest) = path.split_first().ok_or(Error::Unsupported)?;
        let entry = self.entries.get_mut(*name).ok_or(Error::NotFound)?;
        let RootEntry::Dir(dir) = entry else {
            return Err(Error::Unsupported);
        };
        if rest.is_empty() {
            Ok(dir)
        } else {
            dir.directory_mut(rest)
        }
    }
}
