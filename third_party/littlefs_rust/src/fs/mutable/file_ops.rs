impl<D: BlockDevice + 'static> FilesystemMut<D> {
    fn create_root_inline_file_native(&mut self, path: &str, data: &[u8]) -> Result<bool> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;
        if !parents.is_empty() || data.len() > self.fs.inline_threshold() {
            return Ok(false);
        }
        if name.len() > self.fs.info.name_max as usize {
            return Err(Error::NameTooLong);
        }

        let (target, split_now, id) = self.root_create_target(name)?;
        let entries = [
            CommitEntry::new(Tag::new(LFS_TYPE_CREATE, id, 0), &[]),
            CommitEntry::new(
                Tag::new(LFS_TYPE_REG, id, checked_u10(name.len())?),
                name.as_bytes(),
            ),
            CommitEntry::new(
                Tag::new(LFS_TYPE_INLINESTRUCT, id, checked_u10(data.len())?),
                data,
            ),
        ];

        if split_now {
            self.split_pair_with_entries_native(&target, &entries)
        } else {
            self.append_root_chain_update_entries_maybe_relocating(&target, &entries)
        }
    }

    fn create_root_ctz_file_native(&mut self, path: &str, data: &[u8]) -> Result<bool> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;
        if !parents.is_empty() || data.len() <= self.fs.inline_threshold() {
            return Ok(false);
        }
        if name.len() > self.fs.info.name_max as usize {
            return Err(Error::NameTooLong);
        }
        if data.len() > self.fs.info.file_max as usize {
            return Err(Error::FileTooLarge);
        }

        let (target, split_now, id) = self.root_create_target(name)?;
        let (mut allocator, mut blocks) = self.alloc_ctz_blocks_native(data.len())?;
        self.write_ctz_blocks_native(data, &mut blocks, &mut allocator)?;
        let head = blocks.last().copied().ok_or(Error::Corrupt)?;
        let size = u32::try_from(data.len()).map_err(|_| Error::NoSpace)?;
        let mut ctz_payload = Vec::with_capacity(8);
        ctz_payload.extend_from_slice(&head.to_le_bytes());
        ctz_payload.extend_from_slice(&size.to_le_bytes());
        let entries = [
            CommitEntry::new(Tag::new(LFS_TYPE_CREATE, id, 0), &[]),
            CommitEntry::new(
                Tag::new(LFS_TYPE_REG, id, checked_u10(name.len())?),
                name.as_bytes(),
            ),
            CommitEntry::new(Tag::new(LFS_TYPE_CTZSTRUCT, id, 8), &ctz_payload),
        ];
        let root = self.fs.root.clone();
        if !split_now
            && self.should_relocate_pair_before_compaction(&target)?
            && let Some(plan) =
                self.prepare_split_tail_pair_relocation(&root, &target, &entries, &mut allocator)?
        {
            let needs_orphan_repair =
                self.commit_metadata_relocation_plan(plan, &mut allocator)?;
            self.finish_metadata_relocation(allocator, needs_orphan_repair)?;
            return Ok(true);
        }

        // Build the exposing metadata append before mutating data blocks when
        // the target pair still has room. If it is full, the split path below
        // still keeps the same crash shape: program unreachable CTZ blocks and
        // the new tail pair first, then expose both with the final hardtail
        // commit on the previous chain tail.
        let block = match if split_now {
            Err(Error::NoSpace)
        } else if target.pair == self.fs.root.pair {
            self.build_root_append_block(&entries)
        } else {
            self.build_pair_append_block(&target, &entries)
        } {
            Ok(block) => Some(block),
            Err(Error::NoSpace) => None,
            Err(err) => return Err(err),
        };
        if let Some(block) = block {
            self.cache
                .prog(&mut self.device, target.active_block, block.off, &block.data)?;
            self.cache.sync(&mut self.device)?;
        } else {
            self.split_pair_with_entries_using_allocator(&target, &entries, &mut allocator)?;
        }
        self.allocator = allocator;
        self.refresh_after_native_write()?;
        Ok(true)
    }

    fn create_child_inline_file_native(&mut self, path: &str, data: &[u8]) -> Result<bool> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;
        if parents.is_empty() || data.len() > self.fs.inline_threshold() {
            return Ok(false);
        }
        if name.len() > self.fs.info.name_max as usize {
            return Err(Error::NameTooLong);
        }

        let parent_path = parents.join("/");
        let parent = self.fs.resolve_dir(&parent_path)?;
        if self
            .fs
            .files_in_pair_chain(&parent)?
            .iter()
            .any(|file| file.name == *name)
        {
            return Err(Error::AlreadyExists);
        }
        let (target, split_now, files) = self.create_target_in_chain(&parent, name, false)?;
        let id = dir_create_id(&files, name)?;
        let entries = [
            CommitEntry::new(Tag::new(LFS_TYPE_CREATE, id, 0), &[]),
            CommitEntry::new(
                Tag::new(LFS_TYPE_REG, id, checked_u10(name.len())?),
                name.as_bytes(),
            ),
            CommitEntry::new(
                Tag::new(LFS_TYPE_INLINESTRUCT, id, checked_u10(data.len())?),
                data,
            ),
        ];
        if split_now {
            self.split_pair_with_entries_native(&target, &entries)
        } else {
            self.append_child_pair_entries_maybe_relocating(parents, &parent, &target, &entries)
        }
    }

    fn create_child_ctz_file_native(&mut self, path: &str, data: &[u8]) -> Result<bool> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;
        if parents.is_empty() || data.len() <= self.fs.inline_threshold() {
            return Ok(false);
        }
        if name.len() > self.fs.info.name_max as usize {
            return Err(Error::NameTooLong);
        }
        if data.len() > self.fs.info.file_max as usize {
            return Err(Error::FileTooLarge);
        }

        let parent_path = parents.join("/");
        let parent = self.fs.resolve_dir(&parent_path)?;
        if self
            .fs
            .files_in_pair_chain(&parent)?
            .iter()
            .any(|file| file.name == *name)
        {
            return Err(Error::AlreadyExists);
        }
        let (target, split_now, files) = self.create_target_in_chain(&parent, name, false)?;
        let id = dir_create_id(&files, name)?;
        let (mut allocator, mut blocks) = self.alloc_ctz_blocks_native(data.len())?;
        self.write_ctz_blocks_native(data, &mut blocks, &mut allocator)?;
        let head = blocks.last().copied().ok_or(Error::Corrupt)?;
        let size = u32::try_from(data.len()).map_err(|_| Error::NoSpace)?;
        let mut ctz_payload = Vec::with_capacity(8);
        ctz_payload.extend_from_slice(&head.to_le_bytes());
        ctz_payload.extend_from_slice(&size.to_le_bytes());
        let entries = [
            CommitEntry::new(Tag::new(LFS_TYPE_CREATE, id, 0), &[]),
            CommitEntry::new(
                Tag::new(LFS_TYPE_REG, id, checked_u10(name.len())?),
                name.as_bytes(),
            ),
            CommitEntry::new(Tag::new(LFS_TYPE_CTZSTRUCT, id, 8), &ctz_payload),
        ];
        if !split_now && self.should_relocate_pair_before_compaction(&target)? {
            if parents.len() == 1
                && let Some(plan) = self.prepare_root_child_pair_relocation(
                    parents[0],
                    &target,
                    &entries,
                    &mut allocator,
                )?
            {
                let needs_orphan_repair =
                    self.commit_metadata_relocation_plan(plan, &mut allocator)?;
                self.finish_metadata_relocation(allocator, needs_orphan_repair)?;
                self.rebuild_allocator_from_visible_state()?;
                return Ok(true);
            }
            if let Some(plan) =
                self.prepare_split_tail_pair_relocation(&parent, &target, &entries, &mut allocator)?
            {
                let needs_orphan_repair =
                    self.commit_metadata_relocation_plan(plan, &mut allocator)?;
                self.finish_metadata_relocation(allocator, needs_orphan_repair)?;
                self.rebuild_allocator_from_visible_state()?;
                return Ok(true);
            }
        }

        let child_block = match if split_now {
            Err(Error::NoSpace)
        } else {
            self.build_pair_append_block(&target, &entries)
        } {
            Ok(block) => block,
            Err(Error::NoSpace) => {
                self.split_pair_with_entries_using_allocator(&target, &entries, &mut allocator)?;
                self.allocator = allocator;
                self.refresh_after_native_write()?;
                self.rebuild_allocator_from_visible_state()?;
                return Ok(true);
            }
            Err(err) => return Err(err),
        };
        self.cache
            .prog(&mut self.device, target.active_block, child_block.off, &child_block.data)?;
        self.cache.sync(&mut self.device)?;
        self.allocator = allocator;
        self.refresh_after_native_write()?;
        Ok(true)
    }

    fn update_root_inline_file_native(&mut self, path: &str, data: &[u8]) -> Result<bool> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;
        if !parents.is_empty() || data.len() > self.fs.inline_threshold() {
            return Ok(false);
        }

        let (pair, file) = self.find_record_in_pair_chain(&self.fs.root, name)?;
        if file.ty != FileType::File {
            return Err(Error::IsDir);
        }

        let entries = [CommitEntry::new(
            Tag::new(LFS_TYPE_INLINESTRUCT, file.id, checked_u10(data.len())?),
            data,
        )];
        self.append_root_chain_update_entries_maybe_relocating(&pair, &entries)?;
        self.rebuild_allocator_from_visible_state()?;
        Ok(true)
    }

    fn update_root_ctz_file_native(&mut self, path: &str, data: &[u8]) -> Result<bool> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;
        // Existing files may need CTZ storage even below the inline threshold.
        // Under metadata pressure C littlefs can fall back from an inline
        // payload that no longer fits in the pair to an 8-byte CTZSTRUCT plus
        // data blocks, so mounted updates must keep this path available for
        // any non-empty replacement.
        if !parents.is_empty() || data.is_empty() {
            return Ok(false);
        }
        if data.len() > self.fs.info.file_max as usize {
            return Err(Error::FileTooLarge);
        }

        let (pair, file) = self.find_record_in_pair_chain(&self.fs.root, name)?;
        if file.ty != FileType::File {
            return Err(Error::IsDir);
        }

        let (mut allocator, mut blocks) = self.alloc_ctz_blocks_native(data.len())?;
        self.write_ctz_blocks_native(data, &mut blocks, &mut allocator)?;
        let head = blocks.last().copied().ok_or(Error::Corrupt)?;
        let size = u32::try_from(data.len()).map_err(|_| Error::NoSpace)?;
        let mut ctz_payload = Vec::with_capacity(8);
        ctz_payload.extend_from_slice(&head.to_le_bytes());
        ctz_payload.extend_from_slice(&size.to_le_bytes());
        let entries = [CommitEntry::new(
            Tag::new(LFS_TYPE_CTZSTRUCT, file.id, 8),
            &ctz_payload,
        )];
        let root = self.fs.root.clone();
        if self.should_relocate_pair_before_compaction(&pair)?
            && let Some(plan) =
                self.prepare_split_tail_pair_relocation(&root, &pair, &entries, &mut allocator)?
        {
            let needs_orphan_repair =
                self.commit_metadata_relocation_plan(plan, &mut allocator)?;
            self.finish_metadata_relocation(allocator, needs_orphan_repair)?;
            self.rebuild_allocator_from_visible_state()?;
            return Ok(true);
        }

        // Old file data remains reachable through the current metadata until
        // this final root commit lands. The newly allocated CTZ blocks are
        // deliberately unreachable during the data-programming phase.
        let root_block = if pair.pair == self.fs.root.pair {
            self.build_root_append_block(&entries)?
        } else {
            self.build_pair_append_block(&pair, &entries)?
        };
        self.cache
            .prog(&mut self.device, pair.active_block, root_block.off, &root_block.data)?;
        self.cache.sync(&mut self.device)?;
        self.allocator = allocator;
        self.refresh_after_native_write()?;
        self.rebuild_allocator_from_visible_state()?;
        Ok(true)
    }

    fn update_child_inline_file_native(&mut self, path: &str, data: &[u8]) -> Result<bool> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;
        if parents.is_empty() || data.len() > self.fs.inline_threshold() {
            return Ok(false);
        }

        let parent_path = parents.join("/");
        let parent = self.fs.resolve_dir(&parent_path)?;
        let (pair, file) = self.find_record_in_pair_chain(&parent, name)?;
        if file.ty != FileType::File {
            return Err(Error::IsDir);
        }

        let entries = [CommitEntry::new(
            Tag::new(LFS_TYPE_INLINESTRUCT, file.id, checked_u10(data.len())?),
            data,
        )];
        self.append_child_pair_entries_maybe_relocating(parents, &parent, &pair, &entries)?;
        self.rebuild_allocator_from_visible_state()?;
        Ok(true)
    }

    fn update_child_ctz_file_native(&mut self, path: &str, data: &[u8]) -> Result<bool> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;
        // See the root update path above: a small logical file can still need
        // CTZ representation when its inline bytes do not fit in the pressured
        // child metadata pair.
        if parents.is_empty() || data.is_empty() {
            return Ok(false);
        }
        if data.len() > self.fs.info.file_max as usize {
            return Err(Error::FileTooLarge);
        }

        let parent_path = parents.join("/");
        let parent = self.fs.resolve_dir(&parent_path)?;
        let (pair, file) = self.find_record_in_pair_chain(&parent, name)?;
        if file.ty != FileType::File {
            return Err(Error::IsDir);
        }

        let (mut allocator, mut blocks) = self.alloc_ctz_blocks_native(data.len())?;
        self.write_ctz_blocks_native(data, &mut blocks, &mut allocator)?;
        let head = blocks.last().copied().ok_or(Error::Corrupt)?;
        let size = u32::try_from(data.len()).map_err(|_| Error::NoSpace)?;
        let mut ctz_payload = Vec::with_capacity(8);
        ctz_payload.extend_from_slice(&head.to_le_bytes());
        ctz_payload.extend_from_slice(&size.to_le_bytes());
        let entries = [CommitEntry::new(
            Tag::new(LFS_TYPE_CTZSTRUCT, file.id, 8),
            &ctz_payload,
        )];
        if self.should_relocate_pair_before_compaction(&pair)? {
            if parents.len() == 1
                && let Some(plan) = self.prepare_root_child_pair_relocation(
                    parents[0],
                    &pair,
                    &entries,
                    &mut allocator,
                )?
            {
                let needs_orphan_repair =
                    self.commit_metadata_relocation_plan(plan, &mut allocator)?;
                self.finish_metadata_relocation(allocator, needs_orphan_repair)?;
                self.rebuild_allocator_from_visible_state()?;
                return Ok(true);
            }
            if let Some(plan) =
                self.prepare_split_tail_pair_relocation(&parent, &pair, &entries, &mut allocator)?
            {
                let needs_orphan_repair =
                    self.commit_metadata_relocation_plan(plan, &mut allocator)?;
                self.finish_metadata_relocation(allocator, needs_orphan_repair)?;
                self.rebuild_allocator_from_visible_state()?;
                return Ok(true);
            }
        }

        let child_block = self.build_pair_append_block(&pair, &entries)?;
        self.cache
            .prog(&mut self.device, pair.active_block, child_block.off, &child_block.data)?;
        self.cache.sync(&mut self.device)?;
        self.allocator = allocator;
        self.refresh_after_native_write()?;
        self.rebuild_allocator_from_visible_state()?;
        Ok(true)
    }
}
