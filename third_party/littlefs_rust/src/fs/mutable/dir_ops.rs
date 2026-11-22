impl<D: BlockDevice + 'static> FilesystemMut<D> {
    fn create_root_dir_native(&mut self, path: &str) -> Result<bool> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;
        if !parents.is_empty() {
            return Ok(false);
        }
        if name.len() > self.fs.info.name_max as usize {
            return Err(Error::NameTooLong);
        }

        let (target, split_now, id) = self.root_create_target(name)?;
        let (allocator, pair) = self.alloc_pair_native()?;
        let mut pair_payload = Vec::with_capacity(8);
        pair_payload.extend_from_slice(&pair[0].to_le_bytes());
        pair_payload.extend_from_slice(&pair[1].to_le_bytes());
        let inherited_tail = Self::soft_thread_tail_or_null(&target)?;
        let mut entries = Vec::new();
        entries.push(CommitEntry::new(Tag::new(LFS_TYPE_CREATE, id, 0), &[]));
        entries.push(CommitEntry::new(
            Tag::new(LFS_TYPE_DIR, id, checked_u10(name.len())?),
            name.as_bytes(),
        ));
        entries.push(CommitEntry::new(
            Tag::new(LFS_TYPE_DIRSTRUCT, id, 8),
            &pair_payload,
        ));
        if !split_now && target.hardtail()?.is_none() {
            entries.push(Self::softtail_entry_for_pair(pair));
        }
        let root = self.fs.root.clone();
        let mut allocator = allocator;
        if !split_now
            && self.should_relocate_pair_before_compaction(&target)?
            && let Some(plan) =
                self.prepare_split_tail_pair_relocation(&root, &target, &entries, &mut allocator)?
        {
            self.commit_dir_pair_then_relocation(pair, allocator, plan)?;
            return Ok(true);
        }

        // As with CTZ data, the child directory pair must be durable before the
        // parent root commit makes it reachable. The alternate side is erased
        // explicitly because a scanned-free block can still contain a stale
        // valid metadata revision from an older, now-unreachable directory.
        let root_block = match if split_now {
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
        let empty_dir_block = self.build_empty_metadata_block_with_tail(1, inherited_tail)?;
        self.cache.erase(&mut self.device, pair[0])?;
        self.cache
            .prog(&mut self.device, pair[0], 0, &empty_dir_block)?;
        self.cache.erase(&mut self.device, pair[1])?;
        if let Some(root_block) = root_block {
            self.cache
                .prog(&mut self.device, target.active_block, root_block.off, &root_block.data)?;
            self.cache.sync(&mut self.device)?;
        } else {
            self.split_pair_with_entries_using_allocator(&target, &entries, &mut allocator)?;
        }
        self.allocator = allocator;
        self.refresh_after_native_write()?;
        Ok(true)
    }

    fn create_child_dir_native(&mut self, path: &str) -> Result<bool> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;
        if parents.is_empty() {
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
        let (allocator, pair) = self.alloc_pair_native()?;
        let mut pair_payload = Vec::with_capacity(8);
        pair_payload.extend_from_slice(&pair[0].to_le_bytes());
        pair_payload.extend_from_slice(&pair[1].to_le_bytes());
        let inherited_tail = Self::soft_thread_tail_or_null(&target)?;
        let mut entries = Vec::new();
        entries.push(CommitEntry::new(Tag::new(LFS_TYPE_CREATE, id, 0), &[]));
        entries.push(CommitEntry::new(
            Tag::new(LFS_TYPE_DIR, id, checked_u10(name.len())?),
            name.as_bytes(),
        ));
        entries.push(CommitEntry::new(
            Tag::new(LFS_TYPE_DIRSTRUCT, id, 8),
            &pair_payload,
        ));
        if !split_now && target.hardtail()?.is_none() {
            entries.push(Self::softtail_entry_for_pair(pair));
        }
        let mut allocator = allocator;
        if !split_now && self.should_relocate_pair_before_compaction(&target)? {
            if parents.len() == 1
                && let Some(plan) = self.prepare_root_child_pair_relocation(
                    parents[0],
                    &target,
                    &entries,
                    &mut allocator,
                )?
            {
                self.commit_dir_pair_then_relocation(pair, allocator, plan)?;
                self.rebuild_allocator_from_visible_state()?;
                return Ok(true);
            }
            if let Some(plan) =
                self.prepare_split_tail_pair_relocation(&parent, &target, &entries, &mut allocator)?
            {
                self.commit_dir_pair_then_relocation(pair, allocator, plan)?;
                self.rebuild_allocator_from_visible_state()?;
                return Ok(true);
            }
        }

        // Child mkdir has the same crash-ordering shape as root mkdir: prepare
        // the new directory pair first, then expose it by appending DIRSTRUCT
        // to the parent directory metadata pair. Snapshots before the final
        // parent commit must still see the old directory tree.
        let parent_block = match if split_now {
            Err(Error::NoSpace)
        } else {
            self.build_pair_append_block(&target, &entries)
        } {
            Ok(block) => block,
            Err(Error::NoSpace) => {
                let empty_dir_block =
                    self.build_empty_metadata_block_with_tail(1, inherited_tail)?;
                self.cache.erase(&mut self.device, pair[0])?;
                self.cache
                    .prog(&mut self.device, pair[0], 0, &empty_dir_block)?;
                self.cache.erase(&mut self.device, pair[1])?;
                self.split_pair_with_entries_using_allocator(&target, &entries, &mut allocator)?;
                self.allocator = allocator;
                self.refresh_after_native_write()?;
                self.rebuild_allocator_from_visible_state()?;
                return Ok(true);
            }
            Err(err) => return Err(err),
        };
        let empty_dir_block = self.build_empty_metadata_block_with_tail(1, inherited_tail)?;
        self.cache.erase(&mut self.device, pair[0])?;
        self.cache
            .prog(&mut self.device, pair[0], 0, &empty_dir_block)?;
        self.cache.erase(&mut self.device, pair[1])?;
        self.cache
            .prog(&mut self.device, target.active_block, parent_block.off, &parent_block.data)?;
        self.cache.sync(&mut self.device)?;
        self.allocator = allocator;
        self.refresh_after_native_write()?;
        Ok(true)
    }

    fn rename_dir_into_own_subtree_native(&mut self, from: &str, to_parts: &[&str]) -> Result<()> {
        let (to_name, to_parents) = to_parts.split_last().ok_or(Error::InvalidPath)?;
        if to_name.len() > self.fs.info.name_max as usize {
            return Err(Error::NameTooLong);
        }

        // Match the surprising littlefs C behavior for this edge case. C first
        // resolves the destination parent while the old subtree is still
        // reachable, creates the destination entry there, and then deletes the
        // old source from its original parent. Once the source is unlinked, the
        // destination inside that subtree is no longer reachable from root, so
        // the visible result is the source subtree disappearing. We implement
        // that final visible behavior directly while preserving C's observable
        // validation: the destination parent must exist, type conflicts are
        // still reported, and a non-empty destination directory is rejected.
        let to_parent_path = to_parents.join("/");
        self.fs.resolve_dir(&to_parent_path)?;
        match self.record_in_parent_with_pair(to_parents, to_name) {
            Ok((_pair, destination)) => {
                if destination.ty != FileType::Dir {
                    return Err(Error::NotDir);
                }
                let destination_path = format!("/{}", to_parts.join("/"));
                if !self.fs.read_dir(&destination_path)?.is_empty() {
                    return Err(Error::NotEmpty);
                }
            }
            Err(Error::NotFound) => {}
            Err(err) => return Err(err),
        }

        self.remove_dir_tree_native(from)?;
        Ok(())
    }

    fn rename_record_native(&mut self, from: &str, to: &str, ty: FileType) -> Result<bool> {
        let from_parts = components(from)?;
        let to_parts = components(to)?;
        let (from_name, from_parents) = from_parts.split_last().ok_or(Error::InvalidPath)?;
        let (to_name, to_parents) = to_parts.split_last().ok_or(Error::InvalidPath)?;
        if to_name.len() > self.fs.info.name_max as usize {
            return Err(Error::NameTooLong);
        }
        if ty == FileType::Dir && to_parts.starts_with(&from_parts) {
            return Err(Error::InvalidPath);
        }
        if from_parents == to_parents {
            return self.rename_record_within_parent_native(from_parents, from_name, to_name, ty);
        }

        let (source_pair, source) = self.record_in_parent_with_pair(from_parents, from_name)?;
        if source.ty != ty {
            return Err(if ty == FileType::File {
                Error::IsDir
            } else {
                Error::NotDir
            });
        }

        let move_state = self.move_state_for_record(&source_pair, source.id);
        match self.record_in_parent_with_pair(to_parents, to_name) {
            Ok((destination_pair, destination)) => {
                if destination.ty != ty {
                    return Err(if ty == FileType::File {
                        Error::IsDir
                    } else {
                        Error::NotDir
                    });
                }
                if ty == FileType::Dir && !self.fs.read_dir(to)?.is_empty() {
                    return Err(Error::NotEmpty);
                }
                self.append_record_replace_to_parent(
                    to_parents,
                    to_name,
                    &source,
                    &destination_pair,
                    &destination,
                    Some(move_state),
                )?;
            }
            Err(Error::NotFound) => {
                self.append_record_create_to_parent(
                    to_parents,
                    to_name,
                    &source,
                    Some(move_state),
                )?;
            }
            Err(err) => return Err(err),
        }
        self.unlink_record_native(from, ty, Some(move_state))?;
        self.clear_threaded_move_state_if_needed()?;
        Ok(true)
    }

    fn rename_record_within_parent_native(
        &mut self,
        parents: &[&str],
        from_name: &str,
        to_name: &str,
        ty: FileType,
    ) -> Result<bool> {
        let (source_pair, source) = self.record_in_parent_with_pair(parents, from_name)?;
        if source.ty != ty {
            return Err(if ty == FileType::File {
                Error::IsDir
            } else {
                Error::NotDir
            });
        }
        let destination = match self.record_in_parent_with_pair(parents, to_name) {
            Ok((pair, record)) => Some((pair, record)),
            Err(Error::NotFound) => None,
            Err(err) => return Err(err),
        };
        if let Some((destination_pair, destination)) = destination {
            if destination.ty != ty {
                return Err(if ty == FileType::File {
                    Error::IsDir
                } else {
                    Error::NotDir
                });
            }
            let destination_path = path_in_parent(parents, to_name);
            if ty == FileType::Dir && !self.fs.read_dir(&destination_path)?.is_empty() {
                return Err(Error::NotEmpty);
            }
            if destination_pair.pair == source_pair.pair && destination.id == source.id {
                return Ok(true);
            }

            // Replacing within the same metadata pair mirrors C littlefs's
            // single `lfs_dir_commit`: remove the destination slot, create the
            // moved file at that id, then delete the original source id. If a
            // split directory has source and destination in different pairs,
            // fall back to the same move-state protocol used across parents.
            if destination_pair.pair == source_pair.pair {
                let mut entries = Vec::new();
                entries.push(CommitEntry::new(
                    Tag::new(LFS_TYPE_DELETE, destination.id, 0),
                    &[],
                ));
                entries.extend(self.record_create_entries(&source, destination.id, to_name)?);
                entries.push(CommitEntry::new(
                    Tag::new(LFS_TYPE_DELETE, source.id, 0),
                    &[],
                ));
                if parents.is_empty() {
                    return self
                        .append_root_chain_update_entries_maybe_relocating(&source_pair, &entries);
                }

                let parent_path = parents.join("/");
                let parent = self.fs.resolve_dir(&parent_path)?;
                return self.append_child_pair_entries_maybe_relocating(
                    parents,
                    &parent,
                    &source_pair,
                    &entries,
                );
            }

            let move_state = self.move_state_for_record(&source_pair, source.id);
            self.append_record_replace_to_parent(
                parents,
                to_name,
                &source,
                &destination_pair,
                &destination,
                Some(move_state),
            )?;
            let parent_path = if parents.is_empty() {
                String::from("/")
            } else {
                format!("/{}", parents.join("/"))
            };
            self.unlink_record_native(&join_path(&parent_path, from_name), ty, Some(move_state))?;
            self.clear_threaded_move_state_if_needed()?;
            return Ok(true);
        }

        if parents.is_empty() {
            let all_files = self.fs.files_in_pair_chain(&self.fs.root)?;
            if all_files.iter().any(|file| file.name == to_name) {
                return Err(Error::AlreadyExists);
            }
            let files = source_pair.files()?;
            let remaining = files
                .iter()
                .filter(|file| file.name != from_name)
                .cloned()
                .collect::<Vec<_>>();
            let new_id = if source_pair.pair == self.fs.root.pair {
                root_create_id(&remaining, to_name)?
            } else {
                dir_create_id(&remaining, to_name)?
            };
            let mut entries = Vec::new();
            entries.push(CommitEntry::new(
                Tag::new(LFS_TYPE_DELETE, source.id, 0),
                &[],
            ));
            entries.extend(self.record_create_entries(&source, new_id, to_name)?);
            return self.append_root_chain_update_entries_maybe_relocating(&source_pair, &entries);
        }

        let parent_path = parents.join("/");
        let parent = self.fs.resolve_dir(&parent_path)?;
        let files = source_pair.files()?;
        let remaining = files
            .iter()
            .filter(|file| file.name != from_name)
            .cloned()
            .collect::<Vec<_>>();
        let new_id = dir_create_id(&remaining, to_name)?;
        let mut entries = Vec::new();
        entries.push(CommitEntry::new(
            Tag::new(LFS_TYPE_DELETE, source.id, 0),
            &[],
        ));
        entries.extend(self.record_create_entries(&source, new_id, to_name)?);
        self.append_child_pair_entries_maybe_relocating(parents, &parent, &source_pair, &entries)
    }

    fn append_record_create_to_parent(
        &mut self,
        parents: &[&str],
        name: &str,
        source: &FileRecord,
        global_delta: Option<GlobalState>,
    ) -> Result<bool> {
        if parents.is_empty() {
            let (target, split_now, id) = self.root_create_target(name)?;
            let mut entries = self.record_create_entries(source, id, name)?;
            if let Some(global_delta) = global_delta {
                entries.push(self.move_state_entry(global_delta));
            }
            return if split_now {
                self.split_pair_with_entries_native(&target, &entries)
            } else {
                self.append_root_chain_update_entries_maybe_relocating(&target, &entries)
            };
        }

        let parent_path = parents.join("/");
        let parent = self.fs.resolve_dir(&parent_path)?;
        if self
            .fs
            .files_in_pair_chain(&parent)?
            .iter()
            .any(|file| file.name == name)
        {
            return Err(Error::AlreadyExists);
        }
        let (target, split_now, files) = self.create_target_in_chain(&parent, name, false)?;
        let id = dir_create_id(&files, name)?;
        let mut entries = self.record_create_entries(source, id, name)?;
        if let Some(global_delta) = global_delta {
            entries.push(self.move_state_entry(global_delta));
        }
        if split_now {
            self.split_pair_with_entries_native(&target, &entries)
        } else {
            self.append_child_pair_entries_maybe_relocating(parents, &parent, &target, &entries)
        }
    }

    fn append_record_replace_to_parent(
        &mut self,
        parents: &[&str],
        name: &str,
        source: &FileRecord,
        destination_pair: &MetadataPair,
        destination: &FileRecord,
        move_state: Option<GlobalState>,
    ) -> Result<bool> {
        let mut entries = Vec::new();
        entries.push(CommitEntry::new(
            Tag::new(LFS_TYPE_DELETE, destination.id, 0),
            &[],
        ));
        entries.extend(self.record_create_entries(source, destination.id, name)?);
        let orphaned_destination = match (source.ty, destination.ty, &destination.data) {
            (FileType::Dir, FileType::Dir, FileData::Directory(pair)) => Some(*pair),
            _ => None,
        };
        if let Some(orphan_pair) = orphaned_destination {
            let desired = move_state.unwrap_or_default().with_orphan_count(1)?;
            let marker = self.global_state_replacement_for_pair(destination_pair, desired)?;
            entries.push(self.move_state_entry(marker));
            let committed = if parents.is_empty() {
                self.append_root_chain_update_entries_maybe_relocating(destination_pair, &entries)?
            } else {
                let parent_path = parents.join("/");
                let parent = self.fs.resolve_dir(&parent_path)?;
                self.append_child_pair_entries_maybe_relocating(
                    parents,
                    &parent,
                    destination_pair,
                    &entries,
                )?
            };
            if !self.drop_orphaned_directory_pair(orphan_pair)?
                && self.fs.global_state().has_orphans()
            {
                let desired = self.fs.global_state().with_orphan_count(0)?;
                let root = self.fs.root.clone();
                let replacement = self.global_state_replacement_for_pair(&root, desired)?;
                let entry = self.move_state_entry(replacement);
                self.append_root_entries_native(core::slice::from_ref(&entry))?;
            }
            return Ok(committed);
        }
        if let Some(move_state) = move_state {
            entries.push(self.move_state_entry(move_state));
        }

        if parents.is_empty() {
            return self
                .append_root_chain_update_entries_maybe_relocating(destination_pair, &entries);
        }

        let parent_path = parents.join("/");
        let parent = self.fs.resolve_dir(&parent_path)?;
        self.append_child_pair_entries_maybe_relocating(
            parents,
            &parent,
            destination_pair,
            &entries,
        )
    }

    fn unlink_record_native(
        &mut self,
        path: &str,
        ty: FileType,
        move_state: Option<GlobalState>,
    ) -> Result<bool> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;
        if parents.is_empty() {
            let (pair, source) = self.find_record_in_pair_chain(&self.fs.root, name)?;
            if source.ty != ty {
                return Err(if ty == FileType::File {
                    Error::IsDir
                } else {
                    Error::NotDir
                });
            }
            let mut entries = Vec::new();
            entries.push(CommitEntry::new(
                Tag::new(LFS_TYPE_DELETE, source.id, 0),
                &[],
            ));
            if move_state.is_some() {
                let replacement =
                    self.global_state_replacement_for_pair(&pair, GlobalState::default())?;
                entries.push(self.move_state_entry(replacement));
            }
            return self.append_root_chain_update_entries_maybe_relocating(&pair, &entries);
        }

        let parent_path = parents.join("/");
        let parent = self.fs.resolve_dir(&parent_path)?;
        let (pair, source) = self.find_record_in_pair_chain(&parent, name)?;
        if source.ty != ty {
            return Err(if ty == FileType::File {
                Error::IsDir
            } else {
                Error::NotDir
            });
        }
        let mut entries = Vec::new();
        entries.push(CommitEntry::new(
            Tag::new(LFS_TYPE_DELETE, source.id, 0),
            &[],
        ));
        if move_state.is_some() {
            let replacement =
                self.global_state_replacement_for_pair(&pair, GlobalState::default())?;
            entries.push(self.move_state_entry(replacement));
        }
        self.append_child_pair_entries_maybe_relocating(parents, &parent, &pair, &entries)
    }

    fn record_in_parent_with_pair(
        &self,
        parents: &[&str],
        name: &str,
    ) -> Result<(MetadataPair, FileRecord)> {
        if parents.is_empty() {
            return self.find_record_in_pair_chain(&self.fs.root, name);
        }
        let parent_path = parents.join("/");
        let parent = self.fs.resolve_dir(&parent_path)?;
        self.find_record_in_pair_chain(&parent, name)
    }
}
