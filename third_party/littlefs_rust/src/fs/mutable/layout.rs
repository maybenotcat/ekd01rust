impl<D: BlockDevice + 'static> FilesystemMut<D> {
    fn compacted_pair_entries(
        &self,
        pair: &MetadataPair,
        extra: &[CommitEntry],
    ) -> Result<Vec<CommitEntry>> {
        let mut out = Vec::new();
        let mut files = pair.files()?;
        files.sort_by_key(|file| file.id);
        for file in &files {
            out.push(CommitEntry::new(Tag::new(LFS_TYPE_CREATE, file.id, 0), &[]));
            let name_type = match file.ty {
                FileType::File => LFS_TYPE_REG,
                FileType::Dir => LFS_TYPE_DIR,
            };
            out.push(CommitEntry::new(
                Tag::new(name_type, file.id, checked_u10(file.name.len())?),
                file.name.as_bytes(),
            ));
            match &file.data {
                FileData::Inline(data) => out.push(CommitEntry::new(
                    Tag::new(LFS_TYPE_INLINESTRUCT, file.id, checked_u10(data.len())?),
                    data,
                )),
                FileData::Ctz { head, size } => {
                    let mut payload = Vec::with_capacity(8);
                    payload.extend_from_slice(&head.to_le_bytes());
                    payload.extend_from_slice(&size.to_le_bytes());
                    out.push(CommitEntry::new(
                        Tag::new(LFS_TYPE_CTZSTRUCT, file.id, 8),
                        &payload,
                    ));
                }
                FileData::Directory(child) => {
                    let mut payload = Vec::with_capacity(8);
                    payload.extend_from_slice(&child[0].to_le_bytes());
                    payload.extend_from_slice(&child[1].to_le_bytes());
                    out.push(CommitEntry::new(
                        Tag::new(LFS_TYPE_DIRSTRUCT, file.id, 8),
                        &payload,
                    ));
                }
            }
            for (attr_type, attr) in &file.attrs {
                out.push(CommitEntry::new(
                    Tag::new(
                        LFS_TYPE_USERATTR + u16::from(*attr_type),
                        file.id,
                        checked_u10(attr.len())?,
                    ),
                    attr,
                ));
            }
        }
        if let Some(tail) = pair.tail()? {
            // Compaction must not silently cut the filesystem-wide thread. The
            // reader intentionally ignores softtails for directory listing, but
            // the allocator and orphan repair rely on this edge surviving.
            out.push(Self::tail_entry(tail));
        }
        self.push_existing_global_state(&mut out, pair)?;
        out.extend(extra.iter().map(|entry| entry.clone()));
        Ok(out)
    }

    fn push_existing_global_state(
        &self,
        out: &mut Vec<CommitEntry>,
        pair: &MetadataPair,
    ) -> Result<()> {
        let state = pair.global_state_delta()?;
        if !state.is_zero() {
            // A metadata pair's latest global-state delta is part of its folded
            // state, just like names, structs, attrs, and tails. Compaction and
            // split compaction must carry it forward unless the triggering
            // commit appends a newer replacement. Without this, later writes to
            // a pair that is balancing a pending move would silently erase half
            // of the XOR and C littlefs would report a pending gstate.
            out.push(self.move_state_entry(state));
        }
        Ok(())
    }

    fn should_split_pair_before_append(&self, pair: &MetadataPair) -> Result<bool> {
        Ok(pair.hardtail()?.is_none()
            && pair.state.off + 128 > self.fs.cfg.block_size
            && !pair.files()?.is_empty())
    }

    fn split_pair_with_entries_native(
        &mut self,
        pair: &MetadataPair,
        entries: &[CommitEntry],
    ) -> Result<bool> {
        let mut allocator = self.allocator.clone();
        self.split_pair_with_entries_using_allocator(pair, entries, &mut allocator)?;
        self.allocator = allocator;
        self.refresh_after_native_write()?;
        Ok(true)
    }

    fn split_pair_with_entries_using_allocator(
        &mut self,
        pair: &MetadataPair,
        entries: &[CommitEntry],
        allocator: &mut BlockAllocator,
    ) -> Result<()> {
        // littlefs splits during compaction, not by leaving the old pair
        // untouched and appending only the triggering create to a skinny tail.
        // Prepare the balanced tail first while it is unreachable, then switch
        // the source pair to its alternate side with a compacted commit that
        // contains the first half of entries plus the hardtail pointer. Until
        // that final program lands, mounts keep using the old active side.
        let alternate = if pair.active_block == pair.pair[0] {
            pair.pair[1]
        } else {
            pair.pair[0]
        };
        let parent_block = loop {
            let tail = allocator.alloc_pair()?;
            let mut tail_payload = Vec::with_capacity(8);
            tail_payload.extend_from_slice(&tail[0].to_le_bytes());
            tail_payload.extend_from_slice(&tail[1].to_le_bytes());
            let (parent_entries, tail_entries) =
                self.balanced_split_entries(pair, entries, &tail_payload)?;
            let parent_block =
                self.build_metadata_block(pair.rev.wrapping_add(1), &parent_entries)?;
            let tail_block = self.build_metadata_block(1, &tail_entries)?;

            let write_tail = (|| -> Result<()> {
                self.cache.erase(&mut self.device, tail[0])?;
                self.cache.prog(&mut self.device, tail[0], 0, &tail_block)?;
                self.cache.erase(&mut self.device, tail[1])?;
                Ok(())
            })();
            match write_tail {
                Ok(()) => break parent_block,
                Err(Error::Corrupt) => {
                    // The newly allocated tail pair is unreachable until the
                    // final parent commit, so a bad-block style failure can be
                    // handled by abandoning this pair and rebuilding the split
                    // payload for a fresh pair. Reserving both halves is
                    // conservative but keeps the allocator from immediately
                    // reusing a pair whose validity is now uncertain.
                    allocator.reserve_bad_block(tail[0])?;
                    allocator.reserve_bad_block(tail[1])?;
                    continue;
                }
                Err(err) => return Err(err),
            }
        };
        self.cache.erase(&mut self.device, alternate)?;
        self.cache
            .prog(&mut self.device, alternate, 0, &parent_block)?;
        self.cache.sync(&mut self.device)?;
        Ok(())
    }

    fn rebalance_split_chain_with_entries(
        &mut self,
        chain_head: &MetadataPair,
        target: &MetadataPair,
        entries: &[CommitEntry],
    ) -> Result<bool> {
        if chain_head.hardtail()?.is_none()
            || entries
                .iter()
                .any(|entry| entry.tag().type3() == LFS_TYPE_MOVESTATE)
        {
            return Err(Error::NoSpace);
        }

        let chain = self.metadata_pair_chain(chain_head)?;
        if !chain.iter().any(|pair| pair.pair == target.pair) {
            return Err(Error::NoSpace);
        }

        let mut records = Vec::new();
        for pair in &chain {
            if pair.pair == target.pair {
                records.extend(self.records_after_entries(pair, entries)?);
            } else {
                records.extend(pair.files()?);
            }
        }
        records.sort_by(|a, b| a.name.cmp(&b.name));

        let final_tail = chain
            .last()
            .and_then(|pair| pair.tail().ok().flatten())
            .filter(|tail| !tail.split && tail.pair != [LFS_NULL, LFS_NULL]);
        let chunks = self.pack_split_chain_records(
            &records,
            chain_head.pair == self.fs.root.pair,
            final_tail,
        )?;
        if chunks.is_empty() {
            return Err(Error::NoSpace);
        }

        let mut allocator = self.allocator.clone();
        let mut pairs = Vec::with_capacity(chunks.len());
        pairs.push(chain_head.pair);
        for _ in 1..chunks.len() {
            pairs.push(allocator.alloc_pair()?);
        }

        for index in (1..chunks.len()).rev() {
            let block = self.build_rebalanced_chunk_block(
                &chunks[index],
                false,
                1,
                pairs.get(index + 1).copied(),
                final_tail.filter(|_| index + 1 == chunks.len()),
            )?;
            let pair = pairs[index];
            self.cache.erase(&mut self.device, pair[0])?;
            self.cache.prog(&mut self.device, pair[0], 0, &block)?;
            self.cache.erase(&mut self.device, pair[1])?;
        }

        let alternate = if chain_head.active_block == chain_head.pair[0] {
            chain_head.pair[1]
        } else {
            chain_head.pair[0]
        };
        let head_block = self.build_rebalanced_chunk_block(
            &chunks[0],
            chain_head.pair == self.fs.root.pair,
            chain_head.rev.wrapping_add(1),
            pairs.get(1).copied(),
            final_tail.filter(|_| chunks.len() == 1),
        )?;
        self.cache.erase(&mut self.device, alternate)?;
        self.cache.prog(&mut self.device, alternate, 0, &head_block)?;
        self.cache.sync(&mut self.device)?;
        self.allocator = allocator;
        self.refresh_after_native_write()?;
        self.rebuild_allocator_from_visible_state()?;
        Ok(true)
    }

    fn metadata_pair_chain(&self, chain_head: &MetadataPair) -> Result<Vec<MetadataPair>> {
        let mut chain = Vec::new();
        let mut current = chain_head.clone();
        loop {
            if chain.iter().any(|pair: &MetadataPair| pair.pair == current.pair) {
                return Err(Error::Corrupt);
            }
            let next = current.hardtail()?;
            chain.push(current);
            match next {
                Some(pair) if pair != [LFS_NULL, LFS_NULL] => {
                    current = self.fs.read_pair(pair)?;
                }
                _ => break,
            }
        }
        Ok(chain)
    }

    fn pack_split_chain_records(
        &self,
        records: &[FileRecord],
        root_head: bool,
        final_tail: Option<MetadataTail>,
    ) -> Result<Vec<Vec<FileRecord>>> {
        let mut chunks = Vec::new();
        let mut start = 0usize;
        while start < records.len() {
            let mut chosen = None;
            for end in ((start + 1)..=records.len()).rev() {
                let has_next = end < records.len();
                let chunk = &records[start..end];
                let entries = self.rebalanced_chunk_entries(
                    chunk,
                    root_head && chunks.is_empty(),
                    has_next.then_some([0, 0]),
                    final_tail.filter(|_| !has_next),
                )?;
                if self.build_metadata_block(1, &entries).is_ok() {
                    chosen = Some(end);
                    break;
                }
            }
            let end = chosen.ok_or(Error::NoSpace)?;
            chunks.push(records[start..end].to_vec());
            start = end;
        }
        Ok(chunks)
    }

    fn build_rebalanced_chunk_block(
        &self,
        records: &[FileRecord],
        root_head: bool,
        rev: u32,
        next_pair: Option<[u32; 2]>,
        final_tail: Option<MetadataTail>,
    ) -> Result<Vec<u8>> {
        let entries = self.rebalanced_chunk_entries(records, root_head, next_pair, final_tail)?;
        self.build_metadata_block(rev, &entries)
    }

    fn rebalanced_chunk_entries(
        &self,
        records: &[FileRecord],
        root_head: bool,
        next_pair: Option<[u32; 2]>,
        final_tail: Option<MetadataTail>,
    ) -> Result<Vec<CommitEntry>> {
        let mut entries = self.entries_for_compacted_records(records, root_head)?;
        if let Some(next) = next_pair {
            entries.push(Self::hardtail_entry_for_pair(next));
        } else if let Some(tail) = final_tail {
            entries.push(Self::tail_entry(tail));
        }
        Ok(entries)
    }

    fn records_after_entries(
        &self,
        pair: &MetadataPair,
        entries: &[CommitEntry],
    ) -> Result<Vec<FileRecord>> {
        let mut records = BTreeMap::<u16, FileRecord>::new();
        for record in pair.files()? {
            records.insert(record.id, record);
        }

        for entry in entries {
            let tag = entry.tag();
            match tag.type3() {
                LFS_TYPE_CREATE => {
                    Self::shift_record_create(&mut records, tag.id());
                    records.insert(
                        tag.id(),
                        FileRecord {
                            id: tag.id(),
                            name: String::new(),
                            ty: FileType::File,
                            data: FileData::Inline(Vec::new()),
                            attrs: BTreeMap::new(),
                        },
                    );
                }
                LFS_TYPE_DELETE => {
                    records.remove(&tag.id());
                    Self::shift_record_delete(&mut records, tag.id());
                }
                LFS_TYPE_REG | LFS_TYPE_DIR => {
                    let record = records.get_mut(&tag.id()).ok_or(Error::Corrupt)?;
                    record.ty = if tag.type3() == LFS_TYPE_REG {
                        FileType::File
                    } else {
                        FileType::Dir
                    };
                    record.name = ascii_string(entry.data())?;
                }
                LFS_TYPE_INLINESTRUCT => {
                    let record = records.get_mut(&tag.id()).ok_or(Error::Corrupt)?;
                    record.data = FileData::Inline(entry.data().to_vec());
                }
                LFS_TYPE_CTZSTRUCT => {
                    if entry.data().len() != 8 {
                        return Err(Error::Corrupt);
                    }
                    let record = records.get_mut(&tag.id()).ok_or(Error::Corrupt)?;
                    record.data = FileData::Ctz {
                        head: le32(&entry.data()[0..4])?,
                        size: le32(&entry.data()[4..8])?,
                    };
                }
                LFS_TYPE_DIRSTRUCT => {
                    if entry.data().len() != 8 {
                        return Err(Error::Corrupt);
                    }
                    let record = records.get_mut(&tag.id()).ok_or(Error::Corrupt)?;
                    record.data = FileData::Directory([
                        le32(&entry.data()[0..4])?,
                        le32(&entry.data()[4..8])?,
                    ]);
                }
                ty if (LFS_TYPE_USERATTR..LFS_TYPE_CREATE).contains(&ty) => {
                    let record = records.get_mut(&tag.id()).ok_or(Error::Corrupt)?;
                    let attr_type = (ty - LFS_TYPE_USERATTR) as u8;
                    if tag.is_delete() {
                        record.attrs.remove(&attr_type);
                    } else {
                        record.attrs.insert(attr_type, entry.data().to_vec());
                    }
                }
                _ => {}
            }
        }

        let mut out = records.into_values().collect::<Vec<_>>();
        for (id, record) in out.iter_mut().enumerate() {
            record.id = u16::try_from(id).map_err(|_| Error::Unsupported)?;
            if record.name.is_empty() {
                return Err(Error::Corrupt);
            }
        }
        Ok(out)
    }

    fn shift_record_create(records: &mut BTreeMap<u16, FileRecord>, id: u16) {
        let keys = records
            .keys()
            .copied()
            .filter(|key| *key >= id)
            .collect::<Vec<_>>();
        for key in keys.into_iter().rev() {
            if let Some(mut value) = records.remove(&key) {
                value.id = key + 1;
                records.insert(key + 1, value);
            }
        }
    }

    fn shift_record_delete(records: &mut BTreeMap<u16, FileRecord>, id: u16) {
        let keys = records
            .keys()
            .copied()
            .filter(|key| *key > id)
            .collect::<Vec<_>>();
        for key in keys {
            if let Some(mut value) = records.remove(&key) {
                value.id = key - 1;
                records.insert(key - 1, value);
            }
        }
    }

    fn balanced_split_entries(
        &self,
        pair: &MetadataPair,
        entries: &[CommitEntry],
        tail_payload: &[u8],
    ) -> Result<(Vec<CommitEntry>, Vec<CommitEntry>)> {
        let mut records = pair.files()?;
        records.extend(self.records_from_create_entries(entries)?);
        records.sort_by(|a, b| a.name.cmp(&b.name));
        if records.len() < 2 {
            return Err(Error::NoSpace);
        }

        let split = records.len() / 2;
        let parent_records = &records[..split];
        let tail_records = &records[split..];
        let parent_is_root = pair.pair == self.fs.root.pair;
        let mut parent_entries =
            self.entries_for_compacted_records(parent_records, parent_is_root)?;
        parent_entries.push(CommitEntry::new(
            Tag::new(LFS_TYPE_HARDTAIL, 0x3ff, 8),
            tail_payload,
        ));
        self.push_existing_global_state(&mut parent_entries, pair)?;
        for entry in entries {
            if entry.tag().type3() == LFS_TYPE_MOVESTATE {
                parent_entries.push(entry.clone());
            }
        }
        let mut tail_entries = self.entries_for_compacted_records(tail_records, false)?;
        if let Some(old_tail) = pair.tail()?
            && old_tail.pair != [LFS_NULL, LFS_NULL]
        {
            // Splitting turns the source pair's old thread edge into the new
            // split tail's edge: head --hardtail--> split-tail --old-tail-->
            // next metadata pair. Without this, a directory split would make
            // later orphan scans stop inside the directory chain.
            tail_entries.push(Self::tail_entry(old_tail));
        }
        Ok((parent_entries, tail_entries))
    }

    fn entries_for_compacted_records(
        &self,
        records: &[FileRecord],
        root_head: bool,
    ) -> Result<Vec<CommitEntry>> {
        let mut out = Vec::new();
        if root_head {
            out.push(CommitEntry::new(Tag::new(LFS_TYPE_CREATE, 0, 0), &[]));
            out.push(CommitEntry::new(
                Tag::new(LFS_TYPE_SUPERBLOCK, 0, 8),
                b"littlefs",
            ));
            out.push(CommitEntry::new(
                Tag::new(LFS_TYPE_INLINESTRUCT, 0, 24),
                &self.superblock_payload(),
            ));
        }

        for (index, record) in records.iter().enumerate() {
            let id = if root_head { index + 1 } else { index };
            let id = u16::try_from(id).map_err(|_| Error::Unsupported)?;
            out.extend(self.record_create_entries(record, id, &record.name)?);
        }
        Ok(out)
    }

    fn records_from_create_entries(&self, entries: &[CommitEntry]) -> Result<Vec<FileRecord>> {
        let mut records = BTreeMap::<u16, PendingRecord>::new();
        for entry in entries {
            let tag = entry.tag();
            if tag.type3() == LFS_TYPE_CREATE {
                records.entry(tag.id()).or_default();
                continue;
            }

            let record = records.entry(tag.id()).or_default();
            match tag.type3() {
                LFS_TYPE_REG => {
                    record.ty = Some(FileType::File);
                    record.name = Some(ascii_string(entry.data())?);
                }
                LFS_TYPE_DIR => {
                    record.ty = Some(FileType::Dir);
                    record.name = Some(ascii_string(entry.data())?);
                }
                LFS_TYPE_INLINESTRUCT => {
                    record.data = Some(FileData::Inline(entry.data().to_vec()));
                }
                LFS_TYPE_CTZSTRUCT => {
                    if entry.data().len() != 8 {
                        return Err(Error::Corrupt);
                    }
                    record.data = Some(FileData::Ctz {
                        head: le32(&entry.data()[0..4])?,
                        size: le32(&entry.data()[4..8])?,
                    });
                }
                LFS_TYPE_DIRSTRUCT => {
                    if entry.data().len() != 8 {
                        return Err(Error::Corrupt);
                    }
                    record.data = Some(FileData::Directory([
                        le32(&entry.data()[0..4])?,
                        le32(&entry.data()[4..8])?,
                    ]));
                }
                ty if (LFS_TYPE_USERATTR..LFS_TYPE_CREATE).contains(&ty) => {
                    record
                        .attrs
                        .insert((ty - LFS_TYPE_USERATTR) as u8, entry.data().to_vec());
                }
                _ => {}
            }
        }

        let mut out = Vec::new();
        for (id, record) in records {
            let (Some(name), Some(ty), Some(data)) = (record.name, record.ty, record.data) else {
                return Err(Error::Corrupt);
            };
            out.push(FileRecord {
                id,
                name,
                ty,
                data,
                attrs: record.attrs,
            });
        }
        Ok(out)
    }

    fn find_record_in_pair_chain(
        &self,
        pair: &MetadataPair,
        name: &str,
    ) -> Result<(MetadataPair, FileRecord)> {
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
                Some(next) if next != [LFS_NULL, LFS_NULL] => {
                    current = self.fs.read_pair(next)?;
                }
                _ => return Err(Error::NotFound),
            }
        }
    }

    fn build_pair_append_block(
        &mut self,
        pair: &MetadataPair,
        entries: &[CommitEntry],
    ) -> Result<MetadataProgram> {
        let mut block = alloc::vec![0xff; self.fs.cfg.block_size];
        self.cache
            .read(&self.device, pair.active_block, 0, &mut block)?;
        let mut commit =
            MetadataCommitWriter::append(&mut block, self.fs.options().prog_size, pair.state)?;
        commit.write_entries(entries)?;
        let state = commit.finish()?;
        let data = block[pair.state.off..state.off].to_vec();
        Ok(MetadataProgram {
            off: pair.state.off,
            data,
        })
    }

    fn build_root_append_block(&mut self, entries: &[CommitEntry]) -> Result<MetadataProgram> {
        let mut block = alloc::vec![0xff; self.fs.cfg.block_size];
        self.cache
            .read(&self.device, self.fs.root.active_block, 0, &mut block)?;
        let mut commit = MetadataCommitWriter::append(
            &mut block,
            self.fs.options().prog_size,
            self.fs.root.state,
        )?;
        commit.write_entries(entries)?;
        let state = commit.finish()?;
        let data = block[self.fs.root.state.off..state.off].to_vec();
        Ok(MetadataProgram {
            off: self.fs.root.state.off,
            data,
        })
    }

    fn build_empty_metadata_block(&self, rev: u32) -> Result<Vec<u8>> {
        self.build_metadata_block(rev, &[])
    }

    fn build_empty_metadata_block_with_tail(
        &self,
        rev: u32,
        tail: MetadataTail,
    ) -> Result<Vec<u8>> {
        if tail.pair == [LFS_NULL, LFS_NULL] {
            return self.build_empty_metadata_block(rev);
        }
        let tail = Self::tail_entry(tail);
        self.build_metadata_block(rev, core::slice::from_ref(&tail))
    }

    fn build_metadata_block(&self, rev: u32, entries: &[CommitEntry]) -> Result<Vec<u8>> {
        let mut block = alloc::vec![0xff; self.fs.cfg.block_size];
        let mut commit = MetadataCommitWriter::new(&mut block, self.fs.options().prog_size)?;
        commit.write_revision(rev)?;
        commit.write_entries(entries)?;
        let state = commit.finish()?;
        block.truncate(state.off);
        block.shrink_to_fit();
        Ok(block)
    }

    fn alloc_pair_native(&self) -> Result<(BlockAllocator, [u32; 2])> {
        let mut allocator = self.allocator.clone();
        let pair = allocator.alloc_pair()?;
        Ok((allocator, pair))
    }

    fn alloc_ctz_blocks_native(&self, size: usize) -> Result<(BlockAllocator, Vec<u32>)> {
        let mut allocator = self.allocator.clone();
        let blocks = allocator.alloc_ctz_blocks(size)?;
        Ok((allocator, blocks))
    }
}
