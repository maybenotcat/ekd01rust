use super::*;

impl RootEdit {
    pub(super) fn commit_entries(&self) -> Result<Vec<CommitEntry>> {
        match self {
            RootEdit::Storage { id, storage } => Ok(vec![storage_struct_entry(*id, storage)?]),
            RootEdit::Attr {
                id,
                attr_type,
                value,
            } => {
                let tag_type = LFS_TYPE_USERATTR + u16::from(*attr_type);
                let entry = match value {
                    Some(data) => {
                        CommitEntry::new(Tag::new(tag_type, *id, checked_u10(data.len())?), data)
                    }
                    None => CommitEntry::new(Tag::new(tag_type, *id, 0x3ff), &[]),
                };
                Ok(vec![entry])
            }
            RootEdit::Delete { id } => Ok(vec![CommitEntry::new(
                Tag::new(LFS_TYPE_DELETE, *id, 0),
                &[],
            )]),
        }
    }

    pub(super) fn apply_to_root_entries(
        &self,
        entries: &mut BTreeMap<String, RootEntry>,
    ) -> Result<()> {
        let key = root_key_for_id(entries, self.id())?.to_string();
        if matches!(self, RootEdit::Delete { .. }) {
            entries.remove(&key);
            return Ok(());
        }

        let entry = entries.get_mut(&key).ok_or(Error::Corrupt)?;
        let RootEntry::File(file) = entry else {
            return Err(Error::Unsupported);
        };
        match self {
            RootEdit::Storage { storage, .. } => {
                file.storage = storage.clone();
            }
            RootEdit::Attr {
                attr_type, value, ..
            } => match value {
                Some(data) => {
                    file.attrs.insert(*attr_type, data.clone());
                }
                None => {
                    file.attrs.remove(attr_type);
                }
            },
            RootEdit::Delete { .. } => unreachable!("delete edits return before file mutation"),
        }
        Ok(())
    }

    pub(super) fn id(&self) -> u16 {
        match self {
            RootEdit::Storage { id, .. } | RootEdit::Attr { id, .. } => *id,
            RootEdit::Delete { id } => *id,
        }
    }
}

impl FileStorage {
    pub(super) fn write_blocks(&self, image: &mut [u8], cfg: Config) -> Result<()> {
        match self {
            FileStorage::Inline(_) => Ok(()),
            FileStorage::Ctz(ctz) => ctz.write_blocks(image, cfg),
            FileStorage::ExistingCtz { .. } => Ok(()),
        }
    }

    pub(super) fn erase_blocks(&self, image: &mut [u8], cfg: Config) -> Result<()> {
        match self {
            FileStorage::Inline(_) | FileStorage::ExistingCtz { .. } => Ok(()),
            FileStorage::Ctz(ctz) => ctz.erase_blocks(image, cfg),
        }
    }
}

impl Directory {
    pub(super) fn new(pair: [u32; 2], cfg: Config, options: FilesystemOptions) -> Self {
        Self {
            pair,
            cfg,
            options,
            entries: BTreeMap::new(),
            visible_entries: BTreeMap::new(),
            update_commits: Vec::new(),
        }
    }

    pub(super) fn write_empty_pair(&self, image: &mut [u8], cfg: Config) -> Result<()> {
        for entry in self.entries.values() {
            match entry {
                DirectoryEntry::Dir(dir) => dir.write_empty_pair(image, cfg)?,
                DirectoryEntry::File(file) => {
                    file.storage.write_blocks(image, cfg)?;
                }
            }
        }
        for update in &self.update_commits {
            update.write_blocks(image, cfg)?;
        }

        if let Err(err) = self.write_directory_log(image, cfg) {
            if err != Error::NoSpace {
                return Err(err);
            }

            // Child directories have the same pair semantics as the root. If
            // the append log cannot fit in the first side, write the folded
            // visible state to the alternate side with a newer revision. The
            // parent DIRSTRUCT already names both blocks, so C can discover the
            // compacted side without any parent update.
            let compacted = self.compacted_entries()?;
            self.write_directory_entries(image, cfg, self.pair[1], 2, &compacted)?;
        }
        Ok(())
    }

    pub(super) fn write_directory_log(&self, image: &mut [u8], cfg: Config) -> Result<()> {
        let block = image_block_mut(image, cfg, self.pair[0])?;
        let mut state = self.write_initial_commit_to_block(block, cfg, 1, &self.entries)?;
        for update in &self.update_commits {
            state = update.write_into(block, cfg, self.options.prog_size, state)?;
        }
        Ok(())
    }

    pub(super) fn write_directory_entries(
        &self,
        image: &mut [u8],
        cfg: Config,
        block_id: u32,
        rev: u32,
        entries: &BTreeMap<String, DirectoryEntry>,
    ) -> Result<()> {
        let block = image_block_mut(image, cfg, block_id)?;
        self.write_initial_commit_to_block(block, cfg, rev, entries)?;
        Ok(())
    }

    pub(super) fn write_initial_commit_to_block(
        &self,
        block: &mut [u8],
        cfg: Config,
        rev: u32,
        entries_by_name: &BTreeMap<String, DirectoryEntry>,
    ) -> Result<CommitState> {
        if block.len() != cfg.block_size {
            return Err(Error::InvalidConfig);
        }

        // A directory always needs a valid metadata commit so C can fetch the
        // pair. If the directory has no files this is just revision+CCRC; if it
        // has initial entries, those tags live in the child pair.
        let mut commit = MetadataCommitWriter::new(block, self.options.prog_size)?;
        commit.write_revision(rev)?;
        let entries = directory_entries(entries_by_name)?;
        commit.write_entries(&entries)?;
        commit.finish()
    }

    pub(super) fn compacted_entries(&self) -> Result<BTreeMap<String, DirectoryEntry>> {
        let mut entries = self.entries.clone();
        for update in &self.update_commits {
            let key = dir_key_for_id(&entries, update.id)?.to_string();
            if update.delete_file {
                entries.remove(&key);
                continue;
            }

            let entry = entries.get_mut(&key).ok_or(Error::Corrupt)?;
            let DirectoryEntry::File(file) = entry else {
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

    pub(super) fn create_dir(&mut self, name: &str, pair: [u32; 2]) -> Result<()> {
        if self.entries.contains_key(name) {
            return Err(Error::Unsupported);
        }
        self.entries.insert(
            name.to_string(),
            DirectoryEntry::Dir(Directory::new(pair, self.cfg, self.options)),
        );
        self.visible_entries.insert(name.to_string(), RootKind::Dir);
        Ok(())
    }

    pub(super) fn add_inline_file(&mut self, name: &str, data: &[u8]) -> Result<()> {
        if name.len() > self.options.name_max as usize
            || data.len()
                > self
                    .options
                    .inline_threshold(self.cfg, self.options.attr_max)
        {
            return Err(Error::Unsupported);
        }
        if matches!(self.entries.get(name), Some(DirectoryEntry::Dir(_))) {
            return Err(Error::Unsupported);
        }
        self.entries.insert(
            name.to_string(),
            DirectoryEntry::File(InlineFile {
                storage: FileStorage::Inline(data.to_vec()),
                attrs: BTreeMap::new(),
            }),
        );
        self.visible_entries
            .insert(name.to_string(), RootKind::File);
        Ok(())
    }

    pub(super) fn add_ctz_file(&mut self, name: &str, data: &[u8], blocks: Vec<u32>) -> Result<()> {
        if name.len() > self.options.name_max as usize {
            return Err(Error::Unsupported);
        }
        if matches!(self.entries.get(name), Some(DirectoryEntry::Dir(_))) {
            return Err(Error::Unsupported);
        }
        self.entries.insert(
            name.to_string(),
            DirectoryEntry::File(InlineFile {
                storage: FileStorage::Ctz(CtzFile::new(data, blocks)),
                attrs: BTreeMap::new(),
            }),
        );
        self.visible_entries
            .insert(name.to_string(), RootKind::File);
        Ok(())
    }

    pub(super) fn set_attr(&mut self, name: &str, attr_type: u8, data: &[u8]) -> Result<()> {
        if data.len() > self.options.attr_max as usize {
            return Err(Error::Unsupported);
        }
        let entry = self.entries.get_mut(name).ok_or(Error::NotFound)?;
        let DirectoryEntry::File(file) = entry else {
            return Err(Error::Unsupported);
        };
        file.attrs.insert(attr_type, data.to_vec());
        Ok(())
    }

    pub(super) fn update_inline_file(&mut self, name: &str, data: &[u8]) -> Result<()> {
        if data.len()
            > self
                .options
                .inline_threshold(self.cfg, self.options.attr_max)
        {
            return Err(Error::Unsupported);
        }
        self.update_storage(name, FileStorage::Inline(data.to_vec()))
    }

    pub(super) fn update_storage(&mut self, name: &str, storage: FileStorage) -> Result<()> {
        let id = child_file_id(&self.visible_entries, name)?;
        self.update_commits.push(DirUpdateCommit {
            id,
            storage: Some(storage),
            attrs: BTreeMap::new(),
            delete_file: false,
        });
        Ok(())
    }

    pub(super) fn update_attr(&mut self, name: &str, attr_type: u8, data: &[u8]) -> Result<()> {
        if data.len() > self.options.attr_max as usize {
            return Err(Error::Unsupported);
        }
        let id = child_file_id(&self.visible_entries, name)?;
        let mut attrs = BTreeMap::new();
        attrs.insert(attr_type, Some(data.to_vec()));
        self.update_commits.push(DirUpdateCommit {
            id,
            storage: None,
            attrs,
            delete_file: false,
        });
        Ok(())
    }

    pub(super) fn delete_attr(&mut self, name: &str, attr_type: u8) -> Result<()> {
        let id = child_file_id(&self.visible_entries, name)?;
        let mut attrs = BTreeMap::new();
        attrs.insert(attr_type, None);
        self.update_commits.push(DirUpdateCommit {
            id,
            storage: None,
            attrs,
            delete_file: false,
        });
        Ok(())
    }

    pub(super) fn delete_file(&mut self, name: &str) -> Result<()> {
        let id = child_file_id(&self.visible_entries, name)?;
        if self.visible_entries.remove(name).is_none() {
            return Err(Error::NotFound);
        }
        self.update_commits.push(DirUpdateCommit {
            id,
            storage: None,
            attrs: BTreeMap::new(),
            delete_file: true,
        });
        Ok(())
    }

    pub(super) fn delete_dir(&mut self, name: &str) -> Result<()> {
        let id = child_dir_id(&self.visible_entries, name)?;
        let DirectoryEntry::Dir(dir) = self.entries.get(name).ok_or(Error::Corrupt)? else {
            return Err(Error::Unsupported);
        };
        if !dir.is_empty_for_delete() {
            return Err(Error::Unsupported);
        }
        if self.visible_entries.remove(name).is_none() {
            return Err(Error::NotFound);
        }
        self.update_commits.push(DirUpdateCommit {
            id,
            storage: None,
            attrs: BTreeMap::new(),
            delete_file: true,
        });
        Ok(())
    }

    pub(super) fn is_empty_for_delete(&self) -> bool {
        self.visible_entries.is_empty()
    }

    pub(super) fn directory(&self, path: &[&str]) -> Result<&Directory> {
        let (name, rest) = path.split_first().ok_or(Error::Unsupported)?;
        let entry = self.entries.get(*name).ok_or(Error::NotFound)?;
        let DirectoryEntry::Dir(dir) = entry else {
            return Err(Error::Unsupported);
        };
        if rest.is_empty() {
            Ok(dir)
        } else {
            dir.directory(rest)
        }
    }

    pub(super) fn directory_mut(&mut self, path: &[&str]) -> Result<&mut Directory> {
        let (name, rest) = path.split_first().ok_or(Error::Unsupported)?;
        let entry = self.entries.get_mut(*name).ok_or(Error::NotFound)?;
        let DirectoryEntry::Dir(dir) = entry else {
            return Err(Error::Unsupported);
        };
        if rest.is_empty() {
            Ok(dir)
        } else {
            dir.directory_mut(rest)
        }
    }
}
