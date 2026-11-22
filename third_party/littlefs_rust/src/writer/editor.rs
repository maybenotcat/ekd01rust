use super::*;

impl ImageEditor {
    pub fn open(image: Vec<u8>, cfg: Config) -> Result<Self> {
        let fs = Filesystem::mount(&image, cfg)?;
        let used_blocks = fs.used_blocks()?;
        let root = MetadataPair::read(&image, cfg, [0, 1])?;
        Ok(Self {
            cfg,
            image,
            root,
            used_blocks,
        })
    }

    pub fn used_blocks(&self) -> &[bool] {
        &self.used_blocks
    }

    /// Creates a new file in an imported image.
    ///
    /// The editor appends a normal littlefs CREATE/name/struct commit to the
    /// target metadata pair, choosing the create id by lexical order so later
    /// entries shift as they would in a directory log written by C. Small files
    /// are stored inline; larger files allocate CTZ blocks from the scanned
    /// reachable-block map and erase them before programming.
    pub fn create_file(&mut self, path: &str, data: &[u8]) -> Result<&mut Self> {
        let parts = components(path)?;
        let (name, parents) = split_parent(&parts)?;
        if !parents.is_empty() {
            return self.create_file_in_directory(parents, name, data);
        }
        if name.len() > DEFAULT_NAME_MAX as usize {
            return Err(Error::Unsupported);
        }

        let files = self.root.files()?;
        if files.iter().any(|file| file.name == name) {
            return Err(Error::Unsupported);
        }
        let id = root_create_id(&files, name)?;
        let name_entry = CommitEntry::new(
            Tag::new(LFS_TYPE_REG, id, checked_u10(name.len())?),
            name.as_bytes(),
        );

        if data.len() <= DEFAULT_ATTR_MAX as usize {
            let entries = [
                CommitEntry::new(Tag::new(LFS_TYPE_CREATE, id, 0), &[]),
                name_entry,
                CommitEntry::new(
                    Tag::new(LFS_TYPE_INLINESTRUCT, id, checked_u10(data.len())?),
                    data,
                ),
            ];
            self.append_root_entries(&entries)?;
            return Ok(self);
        }

        let mut allocator = FreshAllocator::from_used(self.cfg, self.used_blocks.clone())?;
        let blocks = allocator.alloc_ctz_blocks(data.len())?;
        let storage = FileStorage::Ctz(CtzFile::new(data, blocks));
        storage.erase_blocks(&mut self.image, self.cfg)?;
        storage.write_blocks(&mut self.image, self.cfg)?;
        let entries = [
            CommitEntry::new(Tag::new(LFS_TYPE_CREATE, id, 0), &[]),
            name_entry,
            storage_struct_entry(id, &storage)?,
        ];
        self.append_root_entries(&entries)?;
        self.used_blocks = allocator.into_used();
        Ok(self)
    }

    fn create_file_in_directory(
        &mut self,
        parents: &[&str],
        name: &str,
        data: &[u8],
    ) -> Result<&mut Self> {
        if name.len() > DEFAULT_NAME_MAX as usize {
            return Err(Error::Unsupported);
        }

        let parent = self.resolve_directory(parents)?;
        let files = self.files_in_pair_chain(&parent)?;
        if files.iter().any(|file| file.name == name) {
            return Err(Error::Unsupported);
        }
        let id = dir_create_id(&files, name)?;
        let name_entry = CommitEntry::new(
            Tag::new(LFS_TYPE_REG, id, checked_u10(name.len())?),
            name.as_bytes(),
        );

        if data.len() <= DEFAULT_ATTR_MAX as usize {
            let entries = [
                CommitEntry::new(Tag::new(LFS_TYPE_CREATE, id, 0), &[]),
                name_entry,
                CommitEntry::new(
                    Tag::new(LFS_TYPE_INLINESTRUCT, id, checked_u10(data.len())?),
                    data,
                ),
            ];
            self.append_pair_entries(&parent, &entries)?;
            return Ok(self);
        }

        let mut allocator = FreshAllocator::from_used(self.cfg, self.used_blocks.clone())?;
        let blocks = allocator.alloc_ctz_blocks(data.len())?;
        let storage = FileStorage::Ctz(CtzFile::new(data, blocks));
        storage.erase_blocks(&mut self.image, self.cfg)?;
        storage.write_blocks(&mut self.image, self.cfg)?;
        let entries = [
            CommitEntry::new(Tag::new(LFS_TYPE_CREATE, id, 0), &[]),
            name_entry,
            storage_struct_entry(id, &storage)?,
        ];
        self.append_pair_entries(&parent, &entries)?;
        self.used_blocks = allocator.into_used();
        Ok(self)
    }

    /// Creates a new root-level directory in an imported image.
    ///
    /// The directory receives a freshly allocated metadata pair containing a
    /// valid empty commit, then the root pair gets a CREATE/name/DIRSTRUCT
    /// commit that makes the child reachable. This mirrors the two pieces C
    /// needs to open the directory after remounting.
    pub fn create_dir(&mut self, path: &str) -> Result<&mut Self> {
        let parts = components(path)?;
        let (name, parents) = split_parent(&parts)?;
        if !parents.is_empty() {
            return self.create_dir_in_directory(parents, name);
        }
        if name.len() > DEFAULT_NAME_MAX as usize {
            return Err(Error::Unsupported);
        }

        let files = self.root.files()?;
        if files.iter().any(|file| file.name == name) {
            return Err(Error::Unsupported);
        }
        let id = root_create_id(&files, name)?;
        let mut allocator = FreshAllocator::from_used(self.cfg, self.used_blocks.clone())?;
        let pair = allocator.alloc_pair()?;
        Directory::new(pair, self.cfg, FilesystemOptions::default())
            .write_empty_pair(&mut self.image, self.cfg)?;

        let mut pair_payload = Vec::with_capacity(8);
        pair_payload.extend_from_slice(&pair[0].to_le_bytes());
        pair_payload.extend_from_slice(&pair[1].to_le_bytes());
        let entries = [
            CommitEntry::new(Tag::new(LFS_TYPE_CREATE, id, 0), &[]),
            CommitEntry::new(
                Tag::new(LFS_TYPE_DIR, id, checked_u10(name.len())?),
                name.as_bytes(),
            ),
            CommitEntry::new(Tag::new(LFS_TYPE_DIRSTRUCT, id, 8), &pair_payload),
        ];
        self.append_root_entries(&entries)?;
        self.used_blocks = allocator.into_used();
        Ok(self)
    }

    fn create_dir_in_directory(&mut self, parents: &[&str], name: &str) -> Result<&mut Self> {
        if name.len() > DEFAULT_NAME_MAX as usize {
            return Err(Error::Unsupported);
        }

        let parent = self.resolve_directory(parents)?;
        let files = self.files_in_pair_chain(&parent)?;
        if files.iter().any(|file| file.name == name) {
            return Err(Error::Unsupported);
        }
        let id = dir_create_id(&files, name)?;
        let mut allocator = FreshAllocator::from_used(self.cfg, self.used_blocks.clone())?;
        let pair = allocator.alloc_pair()?;
        Directory::new(pair, self.cfg, FilesystemOptions::default())
            .write_empty_pair(&mut self.image, self.cfg)?;

        let mut pair_payload = Vec::with_capacity(8);
        pair_payload.extend_from_slice(&pair[0].to_le_bytes());
        pair_payload.extend_from_slice(&pair[1].to_le_bytes());
        let entries = [
            CommitEntry::new(Tag::new(LFS_TYPE_CREATE, id, 0), &[]),
            CommitEntry::new(
                Tag::new(LFS_TYPE_DIR, id, checked_u10(name.len())?),
                name.as_bytes(),
            ),
            CommitEntry::new(Tag::new(LFS_TYPE_DIRSTRUCT, id, 8), &pair_payload),
        ];
        self.append_pair_entries(&parent, &entries)?;
        self.used_blocks = allocator.into_used();
        Ok(self)
    }

    /// Appends an inline struct update for an existing root-level file.
    ///
    /// Imported-image editing starts with this narrow operation because it does
    /// not need new data blocks or parent DIRSTRUCT rewrites. The update is
    /// still a real littlefs metadata commit appended to the active root side,
    /// so upstream C must be able to mount the edited image and read the new
    /// bytes.
    pub fn update_inline_file(&mut self, path: &str, data: &[u8]) -> Result<&mut Self> {
        if data.len() > DEFAULT_ATTR_MAX as usize {
            return Err(Error::Unsupported);
        }

        let parts = components(path)?;
        let (name, parents) = split_parent(&parts)?;
        if !parents.is_empty() {
            let parent = self.resolve_directory(parents)?;
            let (pair, id) = self.find_file_in_pair_chain(&parent, name)?;
            let entry = CommitEntry::new(
                Tag::new(LFS_TYPE_INLINESTRUCT, id, checked_u10(data.len())?),
                data,
            );
            self.append_pair_entries(&pair, &[entry])?;
            return Ok(self);
        }

        let id = self.root_file_id_from_name(name)?;
        self.apply_root_edit(RootEdit::Storage {
            id,
            storage: FileStorage::Inline(data.to_vec()),
        })?;
        Ok(self)
    }

    pub fn update_file(&mut self, path: &str, data: &[u8]) -> Result<&mut Self> {
        if data.len() <= DEFAULT_ATTR_MAX as usize {
            return self.update_inline_file(path, data);
        }

        let mut allocator = FreshAllocator::from_used(self.cfg, self.used_blocks.clone())?;
        let blocks = allocator.alloc_ctz_blocks(data.len())?;
        let storage = FileStorage::Ctz(CtzFile::new(data, blocks));
        storage.erase_blocks(&mut self.image, self.cfg)?;
        storage.write_blocks(&mut self.image, self.cfg)?;

        let parts = components(path)?;
        let (name, parents) = split_parent(&parts)?;
        if !parents.is_empty() {
            let parent = self.resolve_directory(parents)?;
            let (pair, id) = self.find_file_in_pair_chain(&parent, name)?;
            let entry = storage_struct_entry(id, &storage)?;
            self.append_pair_entries(&pair, &[entry])?;
            self.used_blocks = allocator.into_used();
            return Ok(self);
        }

        let id = self.root_file_id_from_name(name)?;
        self.apply_root_edit(RootEdit::Storage { id, storage })?;
        self.used_blocks = allocator.into_used();
        Ok(self)
    }

    pub fn update_attr(&mut self, path: &str, attr_type: u8, data: &[u8]) -> Result<&mut Self> {
        if data.len() > DEFAULT_ATTR_MAX as usize {
            return Err(Error::Unsupported);
        }

        let parts = components(path)?;
        let (name, parents) = split_parent(&parts)?;
        if !parents.is_empty() {
            let parent = self.resolve_directory(parents)?;
            let (pair, id) = self.find_file_in_pair_chain(&parent, name)?;
            let entry = CommitEntry::new(
                Tag::new(
                    LFS_TYPE_USERATTR + u16::from(attr_type),
                    id,
                    checked_u10(data.len())?,
                ),
                data,
            );
            self.append_pair_entries(&pair, &[entry])?;
            return Ok(self);
        }

        let id = self.root_file_id(path)?;
        self.apply_root_edit(RootEdit::Attr {
            id,
            attr_type,
            value: Some(data.to_vec()),
        })?;
        Ok(self)
    }

    pub fn delete_attr(&mut self, path: &str, attr_type: u8) -> Result<&mut Self> {
        let parts = components(path)?;
        let (name, parents) = split_parent(&parts)?;
        if !parents.is_empty() {
            let parent = self.resolve_directory(parents)?;
            let (pair, id) = self.find_file_in_pair_chain(&parent, name)?;
            let entry = CommitEntry::new(
                Tag::new(LFS_TYPE_USERATTR + u16::from(attr_type), id, 0x3ff),
                &[],
            );
            self.append_pair_entries(&pair, &[entry])?;
            return Ok(self);
        }

        let id = self.root_file_id(path)?;
        self.apply_root_edit(RootEdit::Attr {
            id,
            attr_type,
            value: None,
        })?;
        Ok(self)
    }

    pub fn delete_file(&mut self, path: &str) -> Result<&mut Self> {
        let parts = components(path)?;
        let (name, parents) = split_parent(&parts)?;
        if !parents.is_empty() {
            let parent = self.resolve_directory(parents)?;
            let (pair, id) = self.find_file_in_pair_chain(&parent, name)?;
            let entry = CommitEntry::new(Tag::new(LFS_TYPE_DELETE, id, 0), &[]);
            self.append_pair_entries(&pair, &[entry])?;
            return Ok(self);
        }

        let id = self.root_file_id(path)?;
        self.apply_root_edit(RootEdit::Delete { id })?;
        Ok(self)
    }

    pub fn delete_dir(&mut self, path: &str) -> Result<&mut Self> {
        let parts = components(path)?;
        let (name, parents) = split_parent(&parts)?;
        if !parents.is_empty() {
            let parent = self.resolve_directory(parents)?;
            let (pair, file) = self.find_file_record_in_pair_chain(&parent, name)?;
            let FileData::Directory(child) = file.data else {
                return Err(Error::Unsupported);
            };
            if file.ty != crate::types::FileType::Dir {
                return Err(Error::Unsupported);
            }
            let child = MetadataPair::read(&self.image, self.cfg, child)?;
            if !self.files_in_pair_chain(&child)?.is_empty() {
                return Err(Error::Unsupported);
            }
            let entry = CommitEntry::new(Tag::new(LFS_TYPE_DELETE, file.id, 0), &[]);
            self.append_pair_entries(&pair, &[entry])?;
            return Ok(self);
        }

        let file = self
            .root
            .files()?
            .into_iter()
            .find(|file| file.name == name)
            .ok_or(Error::NotFound)?;
        let FileData::Directory(child) = file.data else {
            return Err(Error::Unsupported);
        };
        if file.ty != crate::types::FileType::Dir {
            return Err(Error::Unsupported);
        }
        let child = MetadataPair::read(&self.image, self.cfg, child)?;
        if !self.files_in_pair_chain(&child)?.is_empty() {
            return Err(Error::Unsupported);
        }
        self.apply_root_edit(RootEdit::Delete { id: file.id })?;
        Ok(self)
    }

    fn apply_root_edit(&mut self, edit: RootEdit) -> Result<()> {
        let entries = edit.commit_entries()?;
        match self.append_root_entries(&entries) {
            Ok(()) => Ok(()),
            Err(Error::NoSpace) => self.compact_root_with_edit(&edit),
            Err(err) => Err(err),
        }
    }

    fn append_root_entries(&mut self, entries: &[CommitEntry]) -> Result<()> {
        self.append_pair_entries(&self.root.clone(), entries)?;
        self.root = MetadataPair::read(&self.image, self.cfg, [0, 1])?;
        Ok(())
    }

    fn append_pair_entries(&mut self, pair: &MetadataPair, entries: &[CommitEntry]) -> Result<()> {
        let block = image_block_mut(&mut self.image, self.cfg, pair.active_block)?;
        let mut commit = MetadataCommitWriter::append(block, METADATA_PROG_SIZE, pair.state)?;
        commit.write_entries(entries)?;
        commit.finish()?;
        Ok(())
    }

    fn compact_root_with_edit(&mut self, edit: &RootEdit) -> Result<()> {
        let mut entries = self.root_entries_for_compaction()?;
        edit.apply_to_root_entries(&mut entries)?;

        let alternate = if self.root.active_block == self.root.pair[0] {
            self.root.pair[1]
        } else {
            self.root.pair[0]
        };
        {
            let block = image_block_mut(&mut self.image, self.cfg, alternate)?;
            block.fill(0xff);
            let root = RootCommit::from_entries(self.cfg, FilesystemOptions::default(), &entries)?;
            root.write_into_rev(
                block,
                self.cfg,
                METADATA_PROG_SIZE,
                self.root.rev.wrapping_add(1),
            )?;
        }

        self.root = MetadataPair::read(&self.image, self.cfg, [0, 1])?;
        Ok(())
    }

    fn root_entries_for_compaction(&self) -> Result<BTreeMap<String, RootEntry>> {
        let mut entries = BTreeMap::new();
        for file in self.root.files()? {
            match file.data {
                FileData::Inline(data) if file.ty == crate::types::FileType::File => {
                    entries.insert(
                        file.name,
                        RootEntry::File(InlineFile {
                            storage: FileStorage::Inline(data),
                            attrs: file.attrs,
                        }),
                    );
                }
                FileData::Ctz { head, size } if file.ty == crate::types::FileType::File => {
                    entries.insert(
                        file.name,
                        RootEntry::File(InlineFile {
                            storage: FileStorage::ExistingCtz { head, size },
                            attrs: file.attrs,
                        }),
                    );
                }
                FileData::Directory(pair) if file.ty == crate::types::FileType::Dir => {
                    entries.insert(
                        file.name,
                        RootEntry::Dir(Directory::new(
                            pair,
                            self.cfg,
                            FilesystemOptions::default(),
                        )),
                    );
                }
                _ => return Err(Error::Corrupt),
            }
        }
        Ok(entries)
    }

    fn root_file_id(&self, path: &str) -> Result<u16> {
        let parts = components(path)?;
        let (name, parents) = split_parent(&parts)?;
        if !parents.is_empty() {
            return Err(Error::Unsupported);
        }
        self.root_file_id_from_name(name)
    }

    fn root_file_id_from_name(&self, name: &str) -> Result<u16> {
        let file = self
            .root
            .files()?
            .into_iter()
            .find(|file| file.name == name)
            .ok_or(Error::NotFound)?;
        if file.ty != crate::types::FileType::File {
            return Err(Error::Unsupported);
        }
        Ok(file.id)
    }

    fn resolve_directory(&self, path: &[&str]) -> Result<MetadataPair> {
        let mut pair = self.root.clone();
        for component in path {
            let (dir_pair, file) = self.find_file_record_in_pair_chain(&pair, component)?;
            match file.data {
                FileData::Directory(child) if file.ty == crate::types::FileType::Dir => {
                    pair = MetadataPair::read(&self.image, self.cfg, child)?;
                }
                _ => {
                    let _ = dir_pair;
                    return Err(Error::Unsupported);
                }
            }
        }
        Ok(pair)
    }

    fn find_file_in_pair_chain(
        &self,
        pair: &MetadataPair,
        name: &str,
    ) -> Result<(MetadataPair, u16)> {
        let (pair, file) = self.find_file_record_in_pair_chain(pair, name)?;
        if file.ty != crate::types::FileType::File {
            return Err(Error::Unsupported);
        }
        Ok((pair, file.id))
    }

    fn find_file_record_in_pair_chain(
        &self,
        pair: &MetadataPair,
        name: &str,
    ) -> Result<(MetadataPair, crate::metadata::FileRecord)> {
        let mut current = pair.clone();
        let mut seen = Vec::<[u32; 2]>::new();
        loop {
            if seen.contains(&current.pair) {
                return Err(Error::Corrupt);
            }
            seen.push(current.pair);
            if let Some(file) = current.files()?.into_iter().find(|file| file.name == name) {
                return Ok((current, file));
            }
            match current.hardtail()? {
                Some(next) if next != [crate::format::LFS_NULL, crate::format::LFS_NULL] => {
                    current = MetadataPair::read(&self.image, self.cfg, next)?;
                }
                _ => return Err(Error::NotFound),
            }
        }
    }

    fn files_in_pair_chain(&self, pair: &MetadataPair) -> Result<Vec<crate::metadata::FileRecord>> {
        let mut files = Vec::new();
        let mut current = pair.clone();
        let mut seen = Vec::<[u32; 2]>::new();
        loop {
            if seen.contains(&current.pair) {
                return Err(Error::Corrupt);
            }
            seen.push(current.pair);
            files.extend(current.files()?);
            match current.hardtail()? {
                Some(next) if next != [crate::format::LFS_NULL, crate::format::LFS_NULL] => {
                    current = MetadataPair::read(&self.image, self.cfg, next)?;
                }
                _ => return Ok(files),
            }
        }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.image
    }
}
