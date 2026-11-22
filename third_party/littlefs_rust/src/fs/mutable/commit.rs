impl<D: BlockDevice + 'static> FilesystemMut<D> {
    fn append_root_chain_update_entries_native(
        &mut self,
        pair: &MetadataPair,
        entries: &[CommitEntry],
    ) -> Result<bool> {
        if pair.pair == self.fs.root.pair {
            self.append_root_entries_native(entries)
        } else {
            self.append_pair_entries_native(pair, entries)
        }
    }

    fn append_root_chain_update_entries_maybe_relocating(
        &mut self,
        pair: &MetadataPair,
        entries: &[CommitEntry],
    ) -> Result<bool> {
        let root = self.fs.root.clone();
        if self.should_relocate_pair_before_compaction(pair)?
            && self.relocate_split_tail_pair_with_entries(&root, pair, entries)?
        {
            self.rebuild_allocator_from_visible_state()?;
            return Ok(true);
        }
        match self.append_root_chain_update_entries_native(pair, entries) {
            Ok(committed) => Ok(committed),
            Err(Error::NoSpace) => {
                let root = self.fs.root.clone();
                self.rebalance_split_chain_with_entries(&root, pair, entries)
            }
            Err(err) => Err(err),
        }
    }

    fn append_child_pair_entries_maybe_relocating(
        &mut self,
        parents: &[&str],
        parent: &MetadataPair,
        pair: &MetadataPair,
        entries: &[CommitEntry],
    ) -> Result<bool> {
        if self.should_relocate_pair_before_compaction(pair)? {
            if parents.len() == 1
                && self.relocate_root_child_pair_with_entries(parents[0], pair, entries)?
            {
                self.rebuild_allocator_from_visible_state()?;
                return Ok(true);
            }
            if self.relocate_split_tail_pair_with_entries(parent, pair, entries)? {
                self.rebuild_allocator_from_visible_state()?;
                return Ok(true);
            }
        }
        match self.append_pair_entries_native(pair, entries) {
            Ok(committed) => Ok(committed),
            Err(Error::NoSpace) => self.rebalance_split_chain_with_entries(parent, pair, entries),
            Err(err) => Err(err),
        }
    }

    fn append_root_entries_native(&mut self, entries: &[CommitEntry]) -> Result<bool> {
        let block = match self.build_root_append_block(entries) {
            Ok(block) => block,
            Err(Error::NoSpace) => return self.compact_root_head_with_entries_native(entries),
            Err(err) => return Err(err),
        };
        self.cache
            .prog(&mut self.device, self.fs.root.active_block, block.off, &block.data)?;
        self.cache.sync(&mut self.device)?;
        self.refresh_after_native_write()?;
        Ok(true)
    }

    fn append_pair_entries_native(
        &mut self,
        pair: &MetadataPair,
        entries: &[CommitEntry],
    ) -> Result<bool> {
        let block = match self.build_pair_append_block(pair, entries) {
            Ok(block) => block,
            Err(Error::NoSpace) => return self.compact_pair_with_entries_native(pair, entries),
            Err(err) => return Err(err),
        };
        self.cache
            .prog(&mut self.device, pair.active_block, block.off, &block.data)?;
        self.cache.sync(&mut self.device)?;
        self.refresh_after_native_write()?;
        Ok(true)
    }

    fn compact_pair_with_entries_native(
        &mut self,
        pair: &MetadataPair,
        entries: &[CommitEntry],
    ) -> Result<bool> {
        let compacted = self.compacted_pair_entries(pair, entries)?;
        let block = match self.build_metadata_block(pair.rev.wrapping_add(1), &compacted) {
            Ok(block) => block,
            Err(Error::NoSpace) if self.records_from_create_entries(entries).is_ok() => {
                return self.split_pair_with_entries_native(pair, entries);
            }
            Err(err) => return Err(err),
        };
        let alternate = if pair.active_block == pair.pair[0] {
            pair.pair[1]
        } else {
            pair.pair[0]
        };
        self.cache.erase(&mut self.device, alternate)?;
        self.cache.prog(&mut self.device, alternate, 0, &block)?;
        self.cache.sync(&mut self.device)?;
        self.refresh_after_native_write()?;
        Ok(true)
    }

    fn compact_root_head_with_entries_native(&mut self, entries: &[CommitEntry]) -> Result<bool> {
        let compacted = self.compacted_root_head_entries(entries)?;
        let block = self.build_metadata_block(self.fs.root.rev.wrapping_add(1), &compacted)?;
        let alternate = if self.fs.root.active_block == self.fs.root.pair[0] {
            self.fs.root.pair[1]
        } else {
            self.fs.root.pair[0]
        };
        self.cache.erase(&mut self.device, alternate)?;
        self.cache.prog(&mut self.device, alternate, 0, &block)?;
        self.cache.sync(&mut self.device)?;
        self.refresh_after_native_write()?;
        Ok(true)
    }

    fn compacted_root_head_entries(&self, extra: &[CommitEntry]) -> Result<Vec<CommitEntry>> {
        let mut out = Vec::new();
        out.push(CommitEntry::new(Tag::new(LFS_TYPE_CREATE, 0, 0), &[]));
        out.push(CommitEntry::new(
            Tag::new(LFS_TYPE_SUPERBLOCK, 0, 8),
            b"littlefs",
        ));
        out.push(CommitEntry::new(
            Tag::new(LFS_TYPE_INLINESTRUCT, 0, 24),
            &self.superblock_payload(),
        ));

        let mut files = self.fs.root.files()?;
        files.sort_by_key(|file| file.id);
        for file in &files {
            out.extend(self.record_create_entries(file, file.id, &file.name)?);
        }
        if let Some(tail) = self.fs.root.tail()? {
            // Preserve the exact thread edge during compaction. For split root
            // directories this is also the visible hardtail; for unsplit roots
            // it may be a softtail that only matters to orphan/deorphan.
            out.push(Self::tail_entry(tail));
        }
        self.push_existing_global_state(&mut out, &self.fs.root)?;
        out.extend(extra.iter().cloned());
        Ok(out)
    }

    fn superblock_payload(&self) -> Vec<u8> {
        let info = self.fs.info();
        let mut payload = Vec::with_capacity(24);
        for word in [
            info.disk_version,
            info.block_size,
            info.block_count,
            info.name_max,
            info.file_max,
            info.attr_max,
        ] {
            payload.extend_from_slice(&word.to_le_bytes());
        }
        payload
    }

    fn should_relocate_pair_before_compaction(&self, pair: &MetadataPair) -> Result<bool> {
        let Some(block_cycles) = self.block_cycles else {
            return Ok(false);
        };
        if block_cycles == 0 {
            return Ok(false);
        }
        // C littlefs intentionally relocates on a slightly adjusted odd
        // modulus rather than on `rev >= block_cycles`. The `+1 | 1` shape
        // avoids the `block_cycles = 1` infinite-relocation corner case and
        // avoids even periods that would keep selecting the same half of the
        // metadata pair. We test this directly because relocation cadence is
        // part of wear-leveling behavior, not just an internal detail.
        let interval = block_cycles.checked_add(1).ok_or(Error::InvalidConfig)? | 1;
        Ok(pair.rev.wrapping_add(1) % interval == 0 && pair.hardtail()?.is_none())
    }

    fn hardtail_entry_for_pair(pair: [u32; 2]) -> CommitEntry {
        Self::tail_entry(MetadataTail { split: true, pair })
    }

    fn softtail_entry_for_pair(pair: [u32; 2]) -> CommitEntry {
        Self::tail_entry(MetadataTail { split: false, pair })
    }

    fn tail_entry(tail: MetadataTail) -> CommitEntry {
        let mut payload = Vec::with_capacity(8);
        payload.extend_from_slice(&tail.pair[0].to_le_bytes());
        payload.extend_from_slice(&tail.pair[1].to_le_bytes());
        CommitEntry::new(
            Tag::new(
                if tail.split {
                    LFS_TYPE_HARDTAIL
                } else {
                    LFS_TYPE_SOFTTAIL
                },
                0x3ff,
                8,
            ),
            &payload,
        )
    }

    fn thread_tail_or_null(pair: &MetadataPair) -> Result<MetadataTail> {
        // littlefs treats an absent tail as a null softtail. Making that
        // explicit keeps mkdir and deorphan code close to the C implementation:
        // a newly inserted directory inherits its predecessor's old thread
        // successor, even when that successor is the null end marker.
        Ok(pair.tail()?.unwrap_or(MetadataTail {
            split: false,
            pair: [LFS_NULL, LFS_NULL],
        }))
    }

    fn soft_thread_tail_or_null(pair: &MetadataPair) -> Result<MetadataTail> {
        // New directory heads inherit the predecessor's next directory, not a
        // hardtail that points inside the predecessor's own split chain. Full
        // split-parent insertion will later update the actual chain tail; for
        // now, avoiding inherited hardtails keeps child directories from being
        // accidentally linked to their parent's sibling metadata blocks.
        Ok(match pair.tail()? {
            Some(tail) if !tail.split => tail,
            _ => MetadataTail {
                split: false,
                pair: [LFS_NULL, LFS_NULL],
            },
        })
    }

    fn predecessor_in_pair_chain(
        &self,
        chain_head: &MetadataPair,
        target: [u32; 2],
    ) -> Result<Option<MetadataPair>> {
        if chain_head.pair == target {
            return Ok(None);
        }

        let mut current = chain_head.clone();
        let mut seen = Vec::<[u32; 2]>::new();
        loop {
            if seen.contains(&current.pair) {
                return Err(Error::Corrupt);
            }
            seen.push(current.pair);

            let Some(next) = current.hardtail()? else {
                return Ok(None);
            };
            if next == target {
                return Ok(Some(current));
            }
            if next == [LFS_NULL, LFS_NULL] {
                return Err(Error::Corrupt);
            }
            current = self.fs.read_pair(next)?;
        }
    }

    fn build_pair_update_block(
        &mut self,
        pair: &MetadataPair,
        entries: &[CommitEntry],
    ) -> Result<(u32, MetadataProgram)> {
        let append = if pair.pair == self.fs.root.pair {
            self.build_root_append_block(entries)
        } else {
            self.build_pair_append_block(pair, entries)
        };
        match append {
            Ok(block) => Ok((pair.active_block, block)),
            Err(Error::NoSpace) => {
                let compacted = if pair.pair == self.fs.root.pair {
                    self.compacted_root_head_entries(entries)?
                } else {
                    self.compacted_pair_entries(pair, entries)?
                };
                let block = self.build_metadata_block(pair.rev.wrapping_add(1), &compacted)?;
                let alternate = if pair.active_block == pair.pair[0] {
                    pair.pair[1]
                } else {
                    pair.pair[0]
                };
                Ok((
                    alternate,
                    MetadataProgram {
                        off: 0,
                        data: block,
                    },
                ))
            }
            Err(err) => Err(err),
        }
    }

    fn build_hardtail_update_block(
        &mut self,
        pair: &MetadataPair,
        hardtail: &CommitEntry,
    ) -> Result<(u32, MetadataProgram)> {
        self.build_pair_update_block(pair, core::slice::from_ref(hardtail))
    }

    fn relocated_pair_for_new_block(pair: &MetadataPair, new_block: u32) -> [u32; 2] {
        // littlefs relocates one half of a metadata pair, not both blocks at
        // once. Keeping the old active block in the new pair is what lets
        // parent lookup recognize a desynchronized half-orphan after power
        // loss: the stale thread edge and the updated parent DIRSTRUCT still
        // overlap by one block.
        [new_block, pair.active_block]
    }

    fn relocate_split_tail_pair_with_entries(
        &mut self,
        chain_head: &MetadataPair,
        pair: &MetadataPair,
        entries: &[CommitEntry],
    ) -> Result<bool> {
        let mut allocator = self.allocator.clone();
        let Some(plan) =
            self.prepare_split_tail_pair_relocation(chain_head, pair, entries, &mut allocator)?
        else {
            return Ok(false);
        };

        // A split-tail relocation has two visible states. Until the predecessor
        // hardtail update lands, the directory chain still reaches the old tail
        // and must expose old contents. After that pointer update, the chain
        // reaches the relocated pair with the new entries applied.
        let needs_orphan_repair = self.commit_metadata_relocation_plan(plan, &mut allocator)?;
        self.finish_metadata_relocation(allocator, needs_orphan_repair)?;
        Ok(true)
    }

    fn prepare_split_tail_pair_relocation(
        &mut self,
        chain_head: &MetadataPair,
        pair: &MetadataPair,
        entries: &[CommitEntry],
        allocator: &mut BlockAllocator,
    ) -> Result<Option<MetadataRelocationPlan>> {
        if chain_head.pair == pair.pair || pair.hardtail()?.is_some() {
            return Ok(None);
        }

        let Some(predecessor) = self.predecessor_in_pair_chain(chain_head, pair.pair)? else {
            return Ok(None);
        };
        let relocated_program_block = allocator.alloc_block()?;
        let relocated = Self::relocated_pair_for_new_block(pair, relocated_program_block);
        let compacted = self.compacted_pair_entries(pair, entries)?;
        let relocated_block = self.build_metadata_block(pair.rev.wrapping_add(1), &compacted)?;
        let hardtail = Self::hardtail_entry_for_pair(relocated);
        let (pointer_block, pointer_update_block) =
            self.build_hardtail_update_block(&predecessor, &hardtail)?;
        let erase_pointer_block = pointer_block != predecessor.active_block;
        Ok(Some(MetadataRelocationPlan {
            relocated_program_block,
            relocated_active_block: pair.active_block,
            relocated_block,
            pointer_block,
            pointer_update_block,
            erase_pointer_block,
            pointer_update: RelocationPointerUpdate::SplitTail { predecessor },
            needs_orphan_repair: false,
        }))
    }

    fn relocate_root_child_pair_with_entries(
        &mut self,
        root_name: &str,
        pair: &MetadataPair,
        entries: &[CommitEntry],
    ) -> Result<bool> {
        let mut allocator = self.allocator.clone();
        let Some(plan) =
            self.prepare_root_child_pair_relocation(root_name, pair, entries, &mut allocator)?
        else {
            return Ok(false);
        };

        // The relocated metadata pair is prepared while the root still points
        // at the old pair. Only the final root DIRSTRUCT update exposes the new
        // pair, so power-loss snapshots should observe either the old directory
        // contents or the fully relocated contents.
        let needs_orphan_repair = self.commit_metadata_relocation_plan(plan, &mut allocator)?;
        self.finish_metadata_relocation(allocator, needs_orphan_repair)?;
        Ok(true)
    }

    fn prepare_root_child_pair_relocation(
        &mut self,
        root_name: &str,
        pair: &MetadataPair,
        entries: &[CommitEntry],
        allocator: &mut BlockAllocator,
    ) -> Result<Option<MetadataRelocationPlan>> {
        let (parent_pair, root_file) = self.find_record_in_pair_chain(&self.fs.root, root_name)?;
        match root_file.data {
            FileData::Directory(child) if root_file.ty == FileType::Dir && child == pair.pair => {}
            _ => return Ok(None),
        }

        let relocated_program_block = allocator.alloc_block()?;
        let relocated = Self::relocated_pair_for_new_block(pair, relocated_program_block);
        let compacted = self.compacted_pair_entries(pair, entries)?;
        let relocated_block = self.build_metadata_block(pair.rev.wrapping_add(1), &compacted)?;
        let mut relocated_payload = Vec::with_capacity(8);
        relocated_payload.extend_from_slice(&relocated[0].to_le_bytes());
        relocated_payload.extend_from_slice(&relocated[1].to_le_bytes());
        let desired = self.fs.global_state().with_orphan_count(1)?;
        let marker = self.global_state_replacement_for_pair(&parent_pair, desired)?;
        let parent_entries = [
            CommitEntry::new(Tag::new(LFS_TYPE_DIRSTRUCT, root_file.id, 8), &relocated_payload),
            self.move_state_entry(marker),
        ];
        let (pointer_block, pointer_update_block) =
            self.build_pair_update_block(&parent_pair, &parent_entries)?;
        let erase_pointer_block = pointer_block != parent_pair.active_block;
        Ok(Some(MetadataRelocationPlan {
            relocated_program_block,
            relocated_active_block: pair.active_block,
            relocated_block,
            pointer_block,
            pointer_update_block,
            erase_pointer_block,
            pointer_update: RelocationPointerUpdate::RootChild {
                parent_pair,
                entry_id: root_file.id,
                marker,
            },
            needs_orphan_repair: true,
        }))
    }

    fn refresh_relocation_pointer_update(&mut self, plan: &mut MetadataRelocationPlan) -> Result<()> {
        let relocated = [
            plan.relocated_program_block,
            plan.relocated_active_block,
        ];
        let (pointer_block, pointer_update_block) = match &plan.pointer_update {
            RelocationPointerUpdate::SplitTail { predecessor } => {
                let hardtail = Self::hardtail_entry_for_pair(relocated);
                self.build_hardtail_update_block(predecessor, &hardtail)?
            }
            RelocationPointerUpdate::RootChild {
                parent_pair,
                entry_id,
                marker,
            } => {
                let mut relocated_payload = Vec::with_capacity(8);
                relocated_payload.extend_from_slice(&relocated[0].to_le_bytes());
                relocated_payload.extend_from_slice(&relocated[1].to_le_bytes());
                let parent_entries = [
                    CommitEntry::new(Tag::new(LFS_TYPE_DIRSTRUCT, *entry_id, 8), &relocated_payload),
                    self.move_state_entry(*marker),
                ];
                self.build_pair_update_block(parent_pair, &parent_entries)?
            }
        };
        let active_block = match &plan.pointer_update {
            RelocationPointerUpdate::SplitTail { predecessor } => predecessor.active_block,
            RelocationPointerUpdate::RootChild { parent_pair, .. } => parent_pair.active_block,
        };
        plan.pointer_block = pointer_block;
        plan.pointer_update_block = pointer_update_block;
        plan.erase_pointer_block = pointer_block != active_block;
        Ok(())
    }

    fn retry_relocation_with_new_block(
        &mut self,
        plan: &mut MetadataRelocationPlan,
        allocator: &mut BlockAllocator,
    ) -> Result<()> {
        self.cache.invalidate_all();
        allocator.reserve_bad_block(plan.relocated_program_block)?;
        plan.relocated_program_block = allocator.alloc_block()?;
        self.refresh_relocation_pointer_update(plan)
    }

    fn commit_metadata_relocation_plan(
        &mut self,
        mut plan: MetadataRelocationPlan,
        allocator: &mut BlockAllocator,
    ) -> Result<bool> {
        loop {
            match self.device.erase(plan.relocated_program_block) {
                Ok(()) => {}
                Err(Error::Corrupt) => {
                    self.retry_relocation_with_new_block(&mut plan, allocator)?;
                    continue;
                }
                Err(err) => return Err(err),
            }
            match self
                .device
                .prog(plan.relocated_program_block, 0, &plan.relocated_block)
            {
                Ok(()) => break,
                Err(Error::Corrupt) => {
                    self.retry_relocation_with_new_block(&mut plan, allocator)?;
                    continue;
                }
                Err(err) => return Err(err),
            }
        }
        self.cache.invalidate_all();
        if plan.erase_pointer_block {
            self.cache.erase(&mut self.device, plan.pointer_block)?;
        }
        self.cache.prog(
            &mut self.device,
            plan.pointer_block,
            plan.pointer_update_block.off,
            &plan.pointer_update_block.data,
        )?;
        self.cache.sync(&mut self.device)?;
        Ok(plan.needs_orphan_repair)
    }

    fn finish_metadata_relocation(
        &mut self,
        allocator: BlockAllocator,
        needs_orphan_repair: bool,
    ) -> Result<()> {
        self.allocator = allocator;
        self.refresh_after_native_write()?;
        if needs_orphan_repair && self.fs.global_state().has_orphans() {
            self.repair_global_state()?;
            self.refresh()?;
        }
        self.rebuild_allocator_from_visible_state()?;
        Ok(())
    }

    fn commit_dir_pair_then_relocation(
        &mut self,
        pair: [u32; 2],
        allocator: BlockAllocator,
        plan: MetadataRelocationPlan,
    ) -> Result<()> {
        // The new directory pair must be valid before any relocated parent pair
        // can point at it. Until the final relocation pointer update, this pair
        // is still unreachable and crash snapshots should see the old tree.
        let empty_dir_block = self.build_empty_metadata_block(1)?;
        self.cache.erase(&mut self.device, pair[0])?;
        self.cache
            .prog(&mut self.device, pair[0], 0, &empty_dir_block)?;
        self.cache.erase(&mut self.device, pair[1])?;
        let mut allocator = allocator;
        let needs_orphan_repair = self.commit_metadata_relocation_plan(plan, &mut allocator)?;
        self.finish_metadata_relocation(allocator, needs_orphan_repair)
    }
}
