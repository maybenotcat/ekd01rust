impl<D: BlockDevice + 'static> FilesystemMut<D> {
    pub(super) fn finish_streaming_create_file(
        &mut self,
        path: String,
        stream: StreamingWrite,
    ) -> Result<()> {
        self.finish_streaming_file_with_target(path, stream, StreamingTarget::Create)
    }

    pub(super) fn finish_streaming_file(
        &mut self,
        path: String,
        stream: StreamingWrite,
    ) -> Result<()> {
        let target = stream.target;
        self.finish_streaming_file_with_target(path, stream, target)
    }

    fn finish_streaming_file_with_target(
        &mut self,
        path: String,
        mut stream: StreamingWrite,
        target: StreamingTarget,
    ) -> Result<()> {
        if let Some(current) = stream.current.take() {
            self.flush_streaming_block(&mut stream, current)?;
        }
        self.cache.sync(&mut self.device)?;
        match target {
            StreamingTarget::Create => {
                self.create_file_from_allocated_ctz(&path, stream.len, &stream.blocks, stream.allocator)
            }
            StreamingTarget::Replace => {
                self.update_file_from_allocated_ctz(&path, stream.len, &stream.blocks, stream.allocator)
            }
        }
    }

    pub(super) fn flush_streaming_block(
        &mut self,
        stream: &mut StreamingWrite,
        current: StreamingBlock,
    ) -> Result<()> {
        match current.mode {
            StreamingBlockMode::New => {
                let mut current = current;
                loop {
                    // Streaming create blocks are still unreachable from
                    // metadata, so a bad freshly allocated block can be
                    // abandoned exactly like littlefs abandons a corrupt data
                    // block during file relocation: keep it reserved in the
                    // cloned allocator, allocate a replacement, and rebuild the
                    // CTZ chain with the replacement block id.
                    // Use the cache for the erase so any dirty existing-tail
                    // program is flushed before we start writing the fresh
                    // block directly. This preserves the append ordering when
                    // one sync fills an old CTZ tail and then allocates a new
                    // block.
                    match self.cache.erase(&mut self.device, current.block) {
                        Ok(()) => {}
                        Err(Error::Corrupt) => {
                            stream.allocator.reserve_bad_block(current.block)?;
                            current.block = stream.allocator.alloc_block()?;
                            continue;
                        }
                        Err(err) => return Err(err),
                    }
                    self.cache.invalidate_all();
                    match self.device.prog(current.block, 0, &current.bytes) {
                        Ok(()) => break,
                        Err(Error::Corrupt) => {
                            stream.allocator.reserve_bad_block(current.block)?;
                            current.block = stream.allocator.alloc_block()?;
                            continue;
                        }
                        Err(err) => return Err(err),
                    }
                }
                stream.blocks.push(current.block);
            }
            StreamingBlockMode::ExistingTail => {
                self.cache
                    .prog(&mut self.device, current.block, 0, &current.bytes)?;
            }
        }
        Ok(())
    }

    fn create_file_from_allocated_ctz(
        &mut self,
        path: &str,
        size: usize,
        blocks: &[u32],
        allocator: BlockAllocator,
    ) -> Result<()> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;
        if name.len() > self.fs.info.name_max as usize {
            return Err(Error::NameTooLong);
        }
        if size > self.fs.info.file_max as usize {
            return Err(Error::FileTooLarge);
        }
        let head = blocks.last().copied().ok_or(Error::Corrupt)?;
        let size = u32::try_from(size).map_err(|_| Error::NoSpace)?;
        let mut ctz_payload = Vec::with_capacity(8);
        ctz_payload.extend_from_slice(&head.to_le_bytes());
        ctz_payload.extend_from_slice(&size.to_le_bytes());

        if parents.is_empty() {
            let (target, split_now, id) = self.root_create_target(name)?;
            let entries = [
                CommitEntry::new(Tag::new(LFS_TYPE_CREATE, id, 0), &[]),
                CommitEntry::new(
                    Tag::new(LFS_TYPE_REG, id, checked_u10(name.len())?),
                    name.as_bytes(),
                ),
                CommitEntry::new(Tag::new(LFS_TYPE_CTZSTRUCT, id, 8), &ctz_payload),
            ];
            if split_now {
                let mut allocator = allocator;
                self.split_pair_with_entries_using_allocator(&target, &entries, &mut allocator)?;
                self.allocator = allocator;
            } else {
                let root_block = if target.pair == self.fs.root.pair {
                    self.build_root_append_block(&entries)?
                } else {
                    self.build_pair_append_block(&target, &entries)?
                };
                self.cache
                    .prog(&mut self.device, target.active_block, root_block.off, &root_block.data)?;
                self.cache.sync(&mut self.device)?;
                self.allocator = allocator;
            }
            self.refresh_after_native_write()?;
            return Ok(());
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
            CommitEntry::new(Tag::new(LFS_TYPE_CTZSTRUCT, id, 8), &ctz_payload),
        ];

        let mut allocator = allocator;
        let block = match if split_now {
            Err(Error::NoSpace)
        } else {
            self.build_pair_append_block(&target, &entries)
        } {
            Ok(block) => Some(block),
            Err(Error::NoSpace) => {
                self.split_pair_with_entries_using_allocator(&target, &entries, &mut allocator)?;
                None
            }
            Err(err) => return Err(err),
        };
        if let Some(block) = block {
            self.cache
                .prog(&mut self.device, target.active_block, block.off, &block.data)?;
            self.cache.sync(&mut self.device)?;
        }
        self.allocator = allocator;
        self.refresh_after_native_write()?;
        Ok(())
    }

    fn update_file_from_allocated_ctz(
        &mut self,
        path: &str,
        size: usize,
        blocks: &[u32],
        allocator: BlockAllocator,
    ) -> Result<()> {
        if size > self.fs.info.file_max as usize {
            return Err(Error::FileTooLarge);
        }
        let head = blocks.last().copied().ok_or(Error::Corrupt)?;
        let size = u32::try_from(size).map_err(|_| Error::NoSpace)?;
        let mut ctz_payload = Vec::with_capacity(8);
        ctz_payload.extend_from_slice(&head.to_le_bytes());
        ctz_payload.extend_from_slice(&size.to_le_bytes());

        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;
        if parents.is_empty() {
            let (pair, file) = self.find_record_in_pair_chain(&self.fs.root, name)?;
            if file.ty != FileType::File {
                return Err(Error::IsDir);
            }
            let entries = [CommitEntry::new(
                Tag::new(LFS_TYPE_CTZSTRUCT, file.id, 8),
                &ctz_payload,
            )];
            let mut allocator = allocator;
            let root = self.fs.root.clone();
            if self.should_relocate_pair_before_compaction(&pair)?
                && let Some(plan) = self.prepare_split_tail_pair_relocation(
                    &root,
                    &pair,
                    &entries,
                    &mut allocator,
                )?
            {
                let needs_orphan_repair =
                    self.commit_metadata_relocation_plan(plan, &mut allocator)?;
                self.finish_metadata_relocation(allocator, needs_orphan_repair)?;
                return Ok(());
            }

            let block = if pair.pair == self.fs.root.pair {
                self.build_root_append_block(&entries)?
            } else {
                self.build_pair_append_block(&pair, &entries)?
            };
            self.cache
                .prog(&mut self.device, pair.active_block, block.off, &block.data)?;
            self.cache.sync(&mut self.device)?;
            self.allocator = allocator;
            self.refresh_after_native_write()?;
            self.rebuild_allocator_from_visible_state()?;
            return Ok(());
        }

        let parent_path = parents.join("/");
        let parent = self.fs.resolve_dir(&parent_path)?;
        let (pair, file) = self.find_record_in_pair_chain(&parent, name)?;
        if file.ty != FileType::File {
            return Err(Error::IsDir);
        }
        let entries = [CommitEntry::new(
            Tag::new(LFS_TYPE_CTZSTRUCT, file.id, 8),
            &ctz_payload,
        )];
        let mut allocator = allocator;
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
                return Ok(());
            }
            if let Some(plan) =
                self.prepare_split_tail_pair_relocation(&parent, &pair, &entries, &mut allocator)?
            {
                let needs_orphan_repair =
                    self.commit_metadata_relocation_plan(plan, &mut allocator)?;
                self.finish_metadata_relocation(allocator, needs_orphan_repair)?;
                return Ok(());
            }
        }

        let block = self.build_pair_append_block(&pair, &entries)?;
        self.cache
            .prog(&mut self.device, pair.active_block, block.off, &block.data)?;
        self.cache.sync(&mut self.device)?;
        self.allocator = allocator;
        self.refresh_after_native_write()?;
        self.rebuild_allocator_from_visible_state()
    }

    fn streaming_append_state_for_existing_ctz(&self, path: &str) -> Result<Option<StreamingWrite>> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;
        let (_pair, file) = if parents.is_empty() {
            self.find_record_in_pair_chain(&self.fs.root, name)?
        } else {
            let parent_path = parents.join("/");
            let parent = self.fs.resolve_dir(&parent_path)?;
            self.find_record_in_pair_chain(&parent, name)?
        };
        let FileData::Ctz { head, size } = file.data else {
            return Ok(None);
        };
        let blocks = self.ctz_blocks_from_device(head, size)?;
        let full_len = ctz_full_len(blocks.len(), self.fs.cfg.block_size)?;
        if full_len < size as usize {
            return Ok(None);
        }
        let current = if full_len == size as usize {
            None
        } else {
            let tail_index = blocks.len().checked_sub(1).ok_or(Error::Corrupt)?;
            let tail_start = ctz_data_start(tail_index)?;
            let tail_capacity = self
                .fs
                .cfg
                .block_size
                .checked_sub(tail_start)
                .ok_or(Error::InvalidConfig)?;
            let previous_full_len = full_len.checked_sub(tail_capacity).ok_or(Error::Corrupt)?;
            let tail_used = (size as usize)
                .checked_sub(previous_full_len)
                .ok_or(Error::Corrupt)?;
            let off = tail_start.checked_add(tail_used).ok_or(Error::NoSpace)?;
            if off >= self.fs.cfg.block_size {
                None
            } else {
                Some(StreamingBlock {
                    block: blocks[tail_index],
                    bytes: alloc::vec![0xff; self.fs.cfg.block_size],
                    off,
                    mode: StreamingBlockMode::ExistingTail,
                })
            }
        };
        Ok(Some(StreamingWrite {
            allocator: self.allocator.clone(),
            blocks,
            current,
            len: size as usize,
            target: StreamingTarget::Replace,
        }))
    }

    fn partial_overwrite_state_for_existing_ctz(&self, path: &str) -> Result<Option<MergeWrite>> {
        let parts = components(path)?;
        let (name, parents) = parts.split_last().ok_or(Error::InvalidPath)?;
        let (_pair, file) = if parents.is_empty() {
            self.find_record_in_pair_chain(&self.fs.root, name)?
        } else {
            let parent_path = parents.join("/");
            let parent = self.fs.resolve_dir(&parent_path)?;
            self.find_record_in_pair_chain(&parent, name)?
        };
        match file.data {
            FileData::Ctz { size, .. } => Ok(Some(MergeWrite {
                original_len: size as usize,
                patches: Vec::new(),
            })),
            _ => Ok(None),
        }
    }

    fn write_ctz_blocks_native(
        &mut self,
        data: &[u8],
        blocks: &mut [u32],
        allocator: &mut BlockAllocator,
    ) -> Result<()> {
        let mut data_off = 0usize;
        for index in 0..blocks.len() {
            let data_start = ctz_data_start(index)?;
            let capacity = self
                .fs
                .cfg
                .block_size
                .checked_sub(data_start)
                .ok_or(Error::InvalidConfig)?;
            let remaining = data.len().checked_sub(data_off).ok_or(Error::Corrupt)?;
            let chunk_len = core::cmp::min(capacity, remaining);
            let chunk = data
                .get(data_off..data_off + chunk_len)
                .ok_or(Error::Corrupt)?;

            loop {
                let mut block = alloc::vec![0xff; self.fs.cfg.block_size];
                if index > 0 {
                    let skips = index.trailing_zeros() as usize + 1;
                    for skip in 0..skips {
                        let target_index =
                            index.checked_sub(1usize << skip).ok_or(Error::Corrupt)?;
                        let target = blocks.get(target_index).copied().ok_or(Error::Corrupt)?;
                        program_nor_bytes(&mut block, skip * 4, &target.to_le_bytes())?;
                    }
                }

                program_nor_bytes(&mut block, data_start, chunk)?;

                // A scanned-free block may still contain unreachable old data.
                // Erase-before-program keeps NOR semantics honest and matches
                // the reuse rule already covered for ImageEditor CTZ writes.
                //
                // The block is not reachable from metadata yet, so `Corrupt`
                // means the allocation candidate can be treated as a bad block:
                // leave it reserved in the cloned allocator, choose a fresh
                // replacement, and rebuild this CTZ block with the final skip
                // pointers. Ordinary IO errors still propagate so callers can
                // retry the user-visible operation after remount/repair.
                self.cache.invalidate_all();
                match self.device.erase(blocks[index]) {
                    Ok(()) => {}
                    Err(Error::Corrupt) => {
                        allocator.reserve_bad_block(blocks[index])?;
                        blocks[index] = allocator.alloc_block()?;
                        continue;
                    }
                    Err(err) => return Err(err),
                }
                match self.device.prog(blocks[index], 0, &block) {
                    Ok(()) => break,
                    Err(Error::Corrupt) => {
                        allocator.reserve_bad_block(blocks[index])?;
                        blocks[index] = allocator.alloc_block()?;
                        continue;
                    }
                    Err(err) => return Err(err),
                }
            }
            data_off += chunk_len;
        }

        if data_off != data.len() {
            return Err(Error::Corrupt);
        }
        Ok(())
    }
}

fn ctz_full_len(blocks: usize, block_size: usize) -> Result<usize> {
    let mut len = 0usize;
    for index in 0..blocks {
        let data_start = ctz_data_start(index)?;
        len = len
            .checked_add(block_size.checked_sub(data_start).ok_or(Error::InvalidConfig)?)
            .ok_or(Error::NoSpace)?;
    }
    Ok(len)
}
