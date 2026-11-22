impl<D: BlockDevice + 'static> FilesystemMut<D> {
    fn move_state_for_record(&self, pair: &MetadataPair, id: u16) -> GlobalState {
        GlobalState {
            tag: Tag::new(LFS_TYPE_DELETE, id, 0).0,
            pair: pair.pair,
        }
    }

    fn move_state_entry(&self, state: GlobalState) -> CommitEntry {
        let mut payload = Vec::with_capacity(12);
        payload.extend_from_slice(&state.tag.to_le_bytes());
        payload.extend_from_slice(&state.pair[0].to_le_bytes());
        payload.extend_from_slice(&state.pair[1].to_le_bytes());
        CommitEntry::new(Tag::new(LFS_TYPE_MOVESTATE, 0x3ff, 12), &payload)
    }

    fn repair_global_state(&mut self) -> Result<bool> {
        let state = self.fs.global_state();
        if state.is_zero() {
            return Ok(false);
        }

        if let Some(move_delta) = state.move_delta()? {
            self.repair_global_move_state(move_delta)?;
        }

        let state = self.fs.global_state();
        if state.is_zero() {
            return Ok(true);
        }
        if state.has_move() {
            return Err(Error::Unsupported);
        }
        if state.has_orphans() {
            while self.deorphan_one_half_orphan()? {}
            while self.deorphan_one_full_orphan()? {}
            let state = self.fs.global_state();
            if state.has_orphans() {
                // Older development images and unit tests can contain a pure
                // no-op orphan marker without a threaded orphan to drop. Keep
                // accepting those images, but prefer the real threaded cleanup
                // path whenever a full orphan is discoverable.
                self.clear_global_orphan_state(state)?;
            }
            return Ok(true);
        }
        Err(Error::Unsupported)
    }

    fn repair_global_move_state(&mut self, move_delta: GlobalState) -> Result<bool> {
        let move_tag = Tag(move_delta.tag);
        let pair = self.fs.read_pair(move_delta.pair)?;
        let entries = [
            CommitEntry::new(Tag::new(LFS_TYPE_DELETE, move_tag.id(), 0), &[]),
            self.move_state_entry(move_delta),
        ];
        if pair.pair == self.fs.root.pair {
            self.append_root_chain_update_entries_native(&pair, &entries)?;
        } else {
            self.append_pair_entries_native(&pair, &entries)?;
        }
        Ok(true)
    }

    fn clear_global_orphan_state(&mut self, _orphan_delta: GlobalState) -> Result<bool> {
        let root = self.fs.root.clone();
        let replacement = self.global_state_replacement_for_pair(&root, GlobalState::default())?;
        let entry = self.move_state_entry(replacement);
        self.append_root_entries_native(core::slice::from_ref(&entry))
    }

    fn pair_overlaps(a: [u32; 2], b: [u32; 2]) -> bool {
        a[0] == b[0] || a[0] == b[1] || a[1] == b[0] || a[1] == b[1]
    }

    fn pair_is_sync(a: [u32; 2], b: [u32; 2]) -> bool {
        (a[0] == b[0] && a[1] == b[1]) || (a[0] == b[1] && a[1] == b[0])
    }

    fn clear_threaded_move_state_if_needed(&mut self) -> Result<bool> {
        let (state, _) = self.threaded_global_state()?;
        if state.is_zero() || !state.has_move() || state.has_orphans() {
            return Ok(false);
        }
        let root = self.fs.root.clone();
        let replacement = self.global_state_replacement_for_pair(&root, GlobalState::default())?;
        let entry = self.move_state_entry(replacement);
        self.append_root_entries_native(core::slice::from_ref(&entry))
    }

    fn deorphan_one_half_orphan(&mut self) -> Result<bool> {
        if !self.fs.global_state().has_orphans() {
            return Ok(false);
        }

        let mut predecessor = self.fs.root.clone();
        let mut seen = Vec::<[u32; 2]>::new();
        loop {
            if seen.contains(&predecessor.pair) {
                return Err(Error::Corrupt);
            }
            seen.push(predecessor.pair);

            let Some(tail) = predecessor.tail()? else {
                return Ok(false);
            };
            if tail.pair == [LFS_NULL, LFS_NULL] {
                return Ok(false);
            }

            // This is the relocation half-orphan case from C littlefs. The
            // thread predecessor still names the old directory pair, while the
            // visible parent DIRSTRUCT already names a relocated pair that
            // shares one block with it. Repair by making the predecessor's
            // softtail point at the parent's current pair and clearing the
            // orphan count in the same commit.
            if !tail.split
                && let Some((_parent, relocated_pair)) =
                    self.thread_parent_reference(tail.pair, true)?
                && !Self::pair_is_sync(relocated_pair, tail.pair)
            {
                let desired_total = self.fs.global_state().with_orphan_count(0)?;
                let replacement =
                    self.global_state_replacement_for_pair(&predecessor, desired_total)?;
                let entries = [
                    Self::softtail_entry_for_pair(relocated_pair),
                    self.move_state_entry(replacement),
                ];
                if predecessor.pair == self.fs.root.pair {
                    self.append_root_entries_native(&entries)?;
                } else {
                    self.append_pair_entries_native(&predecessor, &entries)?;
                }
                return Ok(true);
            }

            predecessor = self.fs.read_pair(tail.pair)?;
        }
    }

    fn deorphan_one_full_orphan(&mut self) -> Result<bool> {
        if !self.fs.global_state().has_orphans() {
            return Ok(false);
        }

        let mut predecessor = self.fs.root.clone();
        let mut seen = Vec::<[u32; 2]>::new();
        loop {
            if seen.contains(&predecessor.pair) {
                return Err(Error::Corrupt);
            }
            seen.push(predecessor.pair);

            let Some(tail) = predecessor.tail()? else {
                return Ok(false);
            };
            if tail.pair == [LFS_NULL, LFS_NULL] {
                return Ok(false);
            }

            // A hardtail points to another piece of the same logical
            // directory, so it cannot be a full orphan head. A softtail points
            // to the next directory head in the filesystem thread; when no
            // DIRSTRUCT references that head after a crash, C littlefs drops it
            // by making the predecessor skip over it.
            if !tail.split && self.thread_parent_reference(tail.pair, true)?.is_none() {
                let orphan = self.fs.read_pair(tail.pair)?;
                self.drop_threaded_orphan(&predecessor, &orphan)?;
                return Ok(true);
            }

            predecessor = self.fs.read_pair(tail.pair)?;
        }
    }

    fn thread_parent_reference(
        &self,
        child: [u32; 2],
        allow_overlap: bool,
    ) -> Result<Option<(MetadataPair, [u32; 2])>> {
        let mut current = self.fs.root.clone();
        let mut seen = Vec::<[u32; 2]>::new();
        loop {
            if seen.contains(&current.pair) {
                return Err(Error::Corrupt);
            }
            seen.push(current.pair);

            for file in current.files()? {
                let FileData::Directory(pair) = file.data else {
                    continue;
                };
                if file.ty != FileType::Dir {
                    continue;
                }
                let matches = if allow_overlap {
                    Self::pair_overlaps(pair, child)
                } else {
                    Self::pair_is_sync(pair, child)
                };
                if matches {
                    return Ok(Some((current, pair)));
                }
            }

            let Some(tail) = current.tail()? else {
                return Ok(None);
            };
            if tail.pair == [LFS_NULL, LFS_NULL] {
                return Ok(None);
            }
            current = self.fs.read_pair(tail.pair)?;
        }
    }

    fn predecessor_in_thread(&self, target: [u32; 2]) -> Result<Option<MetadataPair>> {
        let mut current = self.fs.root.clone();
        let mut seen = Vec::<[u32; 2]>::new();
        loop {
            if seen.contains(&current.pair) {
                return Err(Error::Corrupt);
            }
            seen.push(current.pair);

            let Some(tail) = current.tail()? else {
                return Ok(None);
            };
            if Self::pair_is_sync(tail.pair, target) {
                return Ok(Some(current));
            }
            if tail.pair == [LFS_NULL, LFS_NULL] {
                return Ok(None);
            }
            current = self.fs.read_pair(tail.pair)?;
        }
    }

    fn drop_threaded_orphan(
        &mut self,
        predecessor: &MetadataPair,
        orphan: &MetadataPair,
    ) -> Result<bool> {
        let inherited_tail = Self::thread_tail_or_null(orphan)?;
        let desired_total = self.fs.global_state().with_orphan_count(0)?;
        let replacement = self.global_state_replacement_for_pair(predecessor, desired_total)?;
        let entries = [
            Self::tail_entry(inherited_tail),
            self.move_state_entry(replacement),
        ];
        if predecessor.pair == self.fs.root.pair {
            self.append_root_entries_native(&entries)
        } else {
            self.append_pair_entries_native(predecessor, &entries)
        }
    }

    fn drop_orphaned_directory_pair(&mut self, orphan_pair: [u32; 2]) -> Result<bool> {
        let Some(predecessor) = self.predecessor_in_thread(orphan_pair)? else {
            return Ok(false);
        };
        let orphan = self.fs.read_pair(orphan_pair)?;
        self.drop_threaded_orphan(&predecessor, &orphan)
    }

    fn global_state_replacement_for_pair(
        &self,
        pair: &MetadataPair,
        desired_total: GlobalState,
    ) -> Result<GlobalState> {
        let (mut aggregate_without_pair, threaded_pairs) = self.threaded_global_state()?;
        if !threaded_pairs.contains(&pair.pair) {
            aggregate_without_pair = self.fs.global_state();
        }
        aggregate_without_pair.xor(pair.global_state_delta()?);
        let mut replacement = desired_total;
        replacement.xor(aggregate_without_pair);
        Ok(replacement)
    }

    fn threaded_global_state(&self) -> Result<(GlobalState, Vec<[u32; 2]>)> {
        let mut state = GlobalState::default();
        let mut pairs = Vec::<[u32; 2]>::new();
        let mut current = self.fs.root.clone();
        loop {
            if pairs.contains(&current.pair) {
                return Err(Error::Corrupt);
            }
            pairs.push(current.pair);
            state.xor(current.global_state_delta()?);

            let Some(tail) = current.tail()? else {
                break;
            };
            if tail.pair == [LFS_NULL, LFS_NULL] {
                break;
            }
            current = self.fs.read_pair(tail.pair)?;
        }
        Ok((state, pairs))
    }
}
