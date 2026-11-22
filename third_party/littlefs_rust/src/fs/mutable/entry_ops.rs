impl<D: BlockDevice + 'static> FilesystemMut<D> {
    fn record_create_entries(
        &self,
        source: &FileRecord,
        new_id: u16,
        new_name: &str,
    ) -> Result<Vec<CommitEntry>> {
        let mut entries = Vec::new();
        entries.push(CommitEntry::new(Tag::new(LFS_TYPE_CREATE, new_id, 0), &[]));
        let name_type = match source.ty {
            FileType::File => LFS_TYPE_REG,
            FileType::Dir => LFS_TYPE_DIR,
        };
        entries.push(CommitEntry::new(
            Tag::new(name_type, new_id, checked_u10(new_name.len())?),
            new_name.as_bytes(),
        ));
        match &source.data {
            FileData::Inline(data) => entries.push(CommitEntry::new(
                Tag::new(LFS_TYPE_INLINESTRUCT, new_id, checked_u10(data.len())?),
                data,
            )),
            FileData::Ctz { head, size } => {
                let mut payload = Vec::with_capacity(8);
                payload.extend_from_slice(&head.to_le_bytes());
                payload.extend_from_slice(&size.to_le_bytes());
                entries.push(CommitEntry::new(
                    Tag::new(LFS_TYPE_CTZSTRUCT, new_id, 8),
                    &payload,
                ));
            }
            FileData::Directory(pair) => {
                let mut payload = Vec::with_capacity(8);
                payload.extend_from_slice(&pair[0].to_le_bytes());
                payload.extend_from_slice(&pair[1].to_le_bytes());
                entries.push(CommitEntry::new(
                    Tag::new(LFS_TYPE_DIRSTRUCT, new_id, 8),
                    &payload,
                ));
            }
        }
        for (attr_type, attr) in &source.attrs {
            entries.push(CommitEntry::new(
                Tag::new(
                    LFS_TYPE_USERATTR + u16::from(*attr_type),
                    new_id,
                    checked_u10(attr.len())?,
                ),
                attr,
            ));
        }
        Ok(entries)
    }

    fn remove_root_file_native(&mut self, path: &str) -> Result<bool> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;
        if !parents.is_empty() {
            return Ok(false);
        }

        let (pair, file) = self.find_record_in_pair_chain(&self.fs.root, name)?;
        if file.ty != FileType::File {
            return Err(Error::IsDir);
        }

        let entries = [CommitEntry::new(Tag::new(LFS_TYPE_DELETE, file.id, 0), &[])];
        self.append_root_chain_update_entries_maybe_relocating(&pair, &entries)?;
        self.rebuild_allocator_from_visible_state()?;
        Ok(true)
    }

    fn remove_root_dir_native(&mut self, path: &str) -> Result<bool> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;
        if !parents.is_empty() {
            return Ok(false);
        }

        let (pair, file) = self.find_record_in_pair_chain(&self.fs.root, name)?;
        if file.ty != FileType::Dir {
            return Err(Error::NotDir);
        }
        let FileData::Directory(orphan_pair) = file.data else {
            return Err(Error::Corrupt);
        };
        if !self.fs.read_dir(path)?.is_empty() {
            return Err(Error::NotEmpty);
        }

        // Directory removal is a two-commit transaction in C littlefs. The
        // parent DELETE first makes the directory unreachable but records one
        // pending orphan in global state. A crash here mounts as "directory is
        // gone, cleanup still pending". The follow-up dir_drop commit patches
        // the metadata-pair thread so allocator scans no longer see the old
        // child pair, and clears the orphan count.
        let orphan_state = GlobalState::orphan_count(1)?;
        let marker = self.global_state_replacement_for_pair(&pair, orphan_state)?;
        let entries = [
            CommitEntry::new(Tag::new(LFS_TYPE_DELETE, file.id, 0), &[]),
            self.move_state_entry(marker),
        ];
        self.append_root_chain_update_entries_maybe_relocating(&pair, &entries)?;
        if !self.drop_orphaned_directory_pair(orphan_pair)? && self.fs.global_state().has_orphans()
        {
            let state = self.fs.global_state();
            self.clear_global_orphan_state(state)?;
        }
        self.rebuild_allocator_from_visible_state()?;
        Ok(true)
    }

    fn remove_child_dir_native(&mut self, path: &str) -> Result<bool> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;
        if parents.is_empty() {
            return Ok(false);
        }

        let parent_path = parents.join("/");
        let parent = self.fs.resolve_dir(&parent_path)?;
        let (pair, file) = self.find_record_in_pair_chain(&parent, name)?;
        if file.ty != FileType::Dir {
            return Err(Error::NotDir);
        }
        let FileData::Directory(orphan_pair) = file.data else {
            return Err(Error::Corrupt);
        };
        if !self.fs.read_dir(path)?.is_empty() {
            return Err(Error::NotEmpty);
        }

        // Match the root rmdir transaction shape: first expose the visible
        // unlink together with a pending orphan count, then drop the old child
        // pair from the filesystem-wide thread and clear that count.
        let orphan_state = GlobalState::orphan_count(1)?;
        let marker = self.global_state_replacement_for_pair(&pair, orphan_state)?;
        let entries = [
            CommitEntry::new(Tag::new(LFS_TYPE_DELETE, file.id, 0), &[]),
            self.move_state_entry(marker),
        ];
        self.append_child_pair_entries_maybe_relocating(parents, &parent, &pair, &entries)?;
        if !self.drop_orphaned_directory_pair(orphan_pair)? && self.fs.global_state().has_orphans()
        {
            let state = self.fs.global_state();
            self.clear_global_orphan_state(state)?;
        }
        self.rebuild_allocator_from_visible_state()?;
        Ok(true)
    }

    fn remove_dir_tree_native(&mut self, path: &str) -> Result<bool> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;
        if parents.is_empty() {
            let (pair, file) = self.find_record_in_pair_chain(&self.fs.root, name)?;
            if file.ty != FileType::Dir {
                return Err(Error::NotDir);
            }
            let FileData::Directory(orphan_pair) = file.data else {
                return Err(Error::Corrupt);
            };
            self.unlink_root_dir_tree_entry(&pair, file.id, orphan_pair)?;
            self.rebuild_allocator_from_visible_state()?;
            return Ok(true);
        }

        let parent_path = parents.join("/");
        let parent = self.fs.resolve_dir(&parent_path)?;
        let (pair, file) = self.find_record_in_pair_chain(&parent, name)?;
        if file.ty != FileType::Dir {
            return Err(Error::NotDir);
        }
        let FileData::Directory(orphan_pair) = file.data else {
            return Err(Error::Corrupt);
        };
        self.unlink_child_dir_tree_entry(parents, &parent, &pair, file.id, orphan_pair)?;
        self.rebuild_allocator_from_visible_state()?;
        Ok(true)
    }

    fn unlink_root_dir_tree_entry(
        &mut self,
        pair: &MetadataPair,
        id: u16,
        orphan_pair: [u32; 2],
    ) -> Result<()> {
        let orphan_state = GlobalState::orphan_count(1)?;
        let marker = self.global_state_replacement_for_pair(pair, orphan_state)?;
        let entries = [
            CommitEntry::new(Tag::new(LFS_TYPE_DELETE, id, 0), &[]),
            self.move_state_entry(marker),
        ];
        self.append_root_chain_update_entries_maybe_relocating(pair, &entries)?;
        if !self.drop_orphaned_directory_pair(orphan_pair)? && self.fs.global_state().has_orphans()
        {
            let state = self.fs.global_state();
            self.clear_global_orphan_state(state)?;
        }
        Ok(())
    }

    fn unlink_child_dir_tree_entry(
        &mut self,
        parents: &[&str],
        parent: &MetadataPair,
        pair: &MetadataPair,
        id: u16,
        orphan_pair: [u32; 2],
    ) -> Result<()> {
        let orphan_state = GlobalState::orphan_count(1)?;
        let marker = self.global_state_replacement_for_pair(pair, orphan_state)?;
        let entries = [
            CommitEntry::new(Tag::new(LFS_TYPE_DELETE, id, 0), &[]),
            self.move_state_entry(marker),
        ];
        self.append_child_pair_entries_maybe_relocating(parents, parent, pair, &entries)?;
        if !self.drop_orphaned_directory_pair(orphan_pair)? && self.fs.global_state().has_orphans()
        {
            let state = self.fs.global_state();
            self.clear_global_orphan_state(state)?;
        }
        Ok(())
    }

    fn remove_child_file_native(&mut self, path: &str) -> Result<bool> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;
        if parents.is_empty() {
            return Ok(false);
        }

        let parent_path = parents.join("/");
        let parent = self.fs.resolve_dir(&parent_path)?;
        let (pair, file) = self.find_record_in_pair_chain(&parent, name)?;
        if file.ty != FileType::File {
            return Err(Error::IsDir);
        }

        let entries = [CommitEntry::new(Tag::new(LFS_TYPE_DELETE, file.id, 0), &[])];
        self.append_child_pair_entries_maybe_relocating(parents, &parent, &pair, &entries)?;
        self.rebuild_allocator_from_visible_state()?;
        Ok(true)
    }

    fn update_root_attr_native(
        &mut self,
        path: &str,
        attr_type: u8,
        value: Option<&[u8]>,
    ) -> Result<bool> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;
        if !parents.is_empty() {
            return Ok(false);
        }
        if value.is_some_and(|data| data.len() > self.fs.info.attr_max as usize) {
            return Err(Error::NoSpace);
        }

        // USERATTR tags are tied to the same metadata id as the target file or
        // directory entry. C's lfs_setattr/lfs_removeattr does not require the
        // attribute to exist beforehand; it only requires that the path resolves
        // to a real entry, so missing attributes are still removed by appending
        // the delete-sized attr tag.
        let (pair, file) = self.find_record_in_pair_chain(&self.fs.root, name)?;
        let (size, data) = match value {
            Some(data) => (checked_u10(data.len())?, data),
            None => (0x3ff, &[][..]),
        };
        let entries = [CommitEntry::new(
            Tag::new(LFS_TYPE_USERATTR + u16::from(attr_type), file.id, size),
            data,
        )];
        self.append_root_chain_update_entries_maybe_relocating(&pair, &entries)
    }

    fn update_child_attr_native(
        &mut self,
        path: &str,
        attr_type: u8,
        value: Option<&[u8]>,
    ) -> Result<bool> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;
        if parents.is_empty() {
            return Ok(false);
        }
        if value.is_some_and(|data| data.len() > self.fs.info.attr_max as usize) {
            return Err(Error::NoSpace);
        }

        let parent_path = parents.join("/");
        let parent = self.fs.resolve_dir(&parent_path)?;
        let (pair, file) = self.find_record_in_pair_chain(&parent, name)?;
        let (size, data) = match value {
            Some(data) => (checked_u10(data.len())?, data),
            None => (0x3ff, &[][..]),
        };
        let entries = [CommitEntry::new(
            Tag::new(LFS_TYPE_USERATTR + u16::from(attr_type), file.id, size),
            data,
        )];
        self.append_child_pair_entries_maybe_relocating(parents, &parent, &pair, &entries)
    }

    fn root_create_target(&self, name: &str) -> Result<(MetadataPair, bool, u16)> {
        let (target, split_now, local_files) =
            self.create_target_in_chain(&self.fs.root, name, true)?;
        let id = if target.pair == self.fs.root.pair && !split_now {
            root_create_id(&local_files, name)?
        } else {
            dir_create_id(&local_files, name)?
        };
        Ok((target, split_now, id))
    }

    fn create_target_in_chain(
        &self,
        pair: &MetadataPair,
        name: &str,
        root_head: bool,
    ) -> Result<(MetadataPair, bool, Vec<FileRecord>)> {
        let mut current = pair.clone();
        let mut seen = Vec::<[u32; 2]>::new();
        loop {
            if seen.contains(&current.pair) {
                return Err(Error::Corrupt);
            }
            seen.push(current.pair);

            let files = current.files()?;
            if files.iter().any(|file| file.name == name) {
                return Err(Error::AlreadyExists);
            }

            let hardtail = current.hardtail()?;
            let belongs_here =
                files.last().is_none_or(|last| name <= last.name.as_str()) || hardtail.is_none();
            if belongs_here {
                let split_now = hardtail.is_none()
                    && self.should_split_pair_before_append(&current)?
                    && !(root_head && current.pair == self.fs.root.pair && files.is_empty());
                let local_files = if split_now { Vec::new() } else { files };
                return Ok((current, split_now, local_files));
            }

            let Some(next) = hardtail else {
                return Err(Error::Corrupt);
            };
            if next == [LFS_NULL, LFS_NULL] {
                return Err(Error::Corrupt);
            }
            current = self.fs.read_pair(next)?;
        }
    }
}
