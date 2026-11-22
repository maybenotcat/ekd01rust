use super::*;

impl DirUpdateCommit {
    pub(super) fn write_blocks(&self, image: &mut [u8], cfg: Config) -> Result<()> {
        if let Some(storage) = &self.storage {
            storage.write_blocks(image, cfg)?;
        }
        Ok(())
    }

    pub(super) fn write_into(
        &self,
        block: &mut [u8],
        cfg: Config,
        prog_size: usize,
        state: CommitState,
    ) -> Result<CommitState> {
        if block.len() != cfg.block_size {
            return Err(Error::InvalidConfig);
        }
        let mut entries = Vec::new();
        if self.delete_file {
            entries.push(CommitEntry::new(Tag::new(LFS_TYPE_DELETE, self.id, 0), &[]));
        }
        if let Some(storage) = &self.storage {
            entries.push(storage_struct_entry(self.id, storage)?);
        }
        for (attr_type, attr) in &self.attrs {
            let tag_type = LFS_TYPE_USERATTR + u16::from(*attr_type);
            match attr {
                Some(attr) => {
                    entries.push(CommitEntry::new(
                        Tag::new(tag_type, self.id, checked_u10(attr.len())?),
                        attr,
                    ));
                }
                None => {
                    entries.push(CommitEntry::new(Tag::new(tag_type, self.id, 0x3ff), &[]));
                }
            }
        }

        let mut commit = MetadataCommitWriter::append(block, prog_size, state)?;
        commit.write_entries(&entries)?;
        commit.finish()
    }
}

impl RootUpdateCommit {
    pub(super) fn write_blocks(&self, image: &mut [u8], cfg: Config) -> Result<()> {
        if let Some(storage) = &self.storage {
            storage.write_blocks(image, cfg)?;
        }
        Ok(())
    }

    pub(super) fn write_into(
        &self,
        block: &mut [u8],
        cfg: Config,
        prog_size: usize,
        state: CommitState,
    ) -> Result<CommitState> {
        if block.len() != cfg.block_size {
            return Err(Error::InvalidConfig);
        }

        let mut entries = Vec::new();
        if self.delete_file {
            entries.push(CommitEntry::new(Tag::new(LFS_TYPE_DELETE, self.id, 0), &[]));
        }
        if let Some(storage) = &self.storage {
            entries.push(storage_struct_entry(self.id, storage)?);
        }
        for (attr_type, attr) in &self.attrs {
            let tag_type = LFS_TYPE_USERATTR + u16::from(*attr_type);
            match attr {
                Some(attr) => {
                    entries.push(CommitEntry::new(
                        Tag::new(tag_type, self.id, checked_u10(attr.len())?),
                        attr,
                    ));
                }
                None => {
                    entries.push(CommitEntry::new(Tag::new(tag_type, self.id, 0x3ff), &[]));
                }
            }
        }

        // Appended commits do not repeat the metadata-block revision. They
        // start at the offset returned by the previous CCRC and use the
        // previous CCRC tag as the XOR base for their first tag.
        let mut commit = MetadataCommitWriter::append(block, prog_size, state)?;
        commit.write_entries(&entries)?;
        commit.finish()
    }
}

impl RootCommit {
    pub(super) fn from_builder(builder: &ImageBuilder) -> Result<Self> {
        Self::from_entries(builder.cfg, builder.options, &builder.entries)
    }

    pub(super) fn from_entries(
        cfg: Config,
        options: FilesystemOptions,
        root_entries: &BTreeMap<String, RootEntry>,
    ) -> Result<Self> {
        let mut entries = Vec::new();

        // The superblock is represented as directory id 0. C format writes an
        // explicit CREATE before the superblock name/struct tags, so we do the
        // same instead of relying on parser leniency.
        entries.push(CommitEntry::new(Tag::new(LFS_TYPE_CREATE, 0, 0), &[]));
        entries.push(CommitEntry::new(
            Tag::new(LFS_TYPE_SUPERBLOCK, 0, 8),
            b"littlefs",
        ));
        entries.push(CommitEntry::new(
            Tag::new(LFS_TYPE_INLINESTRUCT, 0, 24),
            &superblock_payload(cfg, options),
        ));

        for (index, (name, entry)) in root_entries.iter().enumerate() {
            let id = u16::try_from(index + 1).map_err(|_| Error::Unsupported)?;
            if id >= 0x3ff {
                return Err(Error::Unsupported);
            }

            // Root entries start at id 1 because id 0 is permanently occupied
            // by the superblock entry. Files and directories share the same id
            // sequence, so they must be emitted from one sorted map.
            entries.push(CommitEntry::new(Tag::new(LFS_TYPE_CREATE, id, 0), &[]));
            match entry {
                RootEntry::File(file) => {
                    entries.push(CommitEntry::new(
                        Tag::new(LFS_TYPE_REG, id, checked_u10(name.len())?),
                        name.as_bytes(),
                    ));
                    match &file.storage {
                        FileStorage::Inline(data) => {
                            entries.push(CommitEntry::new(
                                Tag::new(LFS_TYPE_INLINESTRUCT, id, checked_u10(data.len())?),
                                data,
                            ));
                        }
                        FileStorage::Ctz(ctz) => {
                            let mut payload = Vec::with_capacity(8);
                            payload.extend_from_slice(&ctz.head()?.to_le_bytes());
                            payload.extend_from_slice(&(ctz.len() as u32).to_le_bytes());
                            entries.push(CommitEntry::new(
                                Tag::new(LFS_TYPE_CTZSTRUCT, id, 8),
                                &payload,
                            ));
                        }
                        FileStorage::ExistingCtz { head, size } => {
                            let mut payload = Vec::with_capacity(8);
                            payload.extend_from_slice(&head.to_le_bytes());
                            payload.extend_from_slice(&size.to_le_bytes());
                            entries.push(CommitEntry::new(
                                Tag::new(LFS_TYPE_CTZSTRUCT, id, 8),
                                &payload,
                            ));
                        }
                    }

                    for (attr_type, attr) in &file.attrs {
                        entries.push(CommitEntry::new(
                            Tag::new(
                                LFS_TYPE_USERATTR + u16::from(*attr_type),
                                id,
                                checked_u10(attr.len())?,
                            ),
                            attr,
                        ));
                    }
                }
                RootEntry::Dir(dir) => {
                    entries.push(CommitEntry::new(
                        Tag::new(LFS_TYPE_DIR, id, checked_u10(name.len())?),
                        name.as_bytes(),
                    ));
                    let mut pair = Vec::with_capacity(8);
                    pair.extend_from_slice(&dir.pair[0].to_le_bytes());
                    pair.extend_from_slice(&dir.pair[1].to_le_bytes());
                    entries.push(CommitEntry::new(Tag::new(LFS_TYPE_DIRSTRUCT, id, 8), &pair));
                }
            }
        }

        Ok(Self { entries })
    }

    pub(super) fn write_into(
        &self,
        block: &mut [u8],
        cfg: Config,
        prog_size: usize,
    ) -> Result<CommitState> {
        self.write_into_rev(block, cfg, prog_size, 1)
    }

    pub(super) fn write_into_rev(
        &self,
        block: &mut [u8],
        cfg: Config,
        prog_size: usize,
        rev: u32,
    ) -> Result<CommitState> {
        if block.len() != cfg.block_size {
            return Err(Error::InvalidConfig);
        }

        // Metadata blocks begin with a little-endian revision count. Revision 1
        // is enough for a fresh image, and using a non-erased value is what lets
        // mount distinguish this side of the pair from block 1's all-0xffs.
        let mut commit = MetadataCommitWriter::new(block, prog_size)?;
        commit.write_revision(rev)?;
        commit.write_entries(&self.entries)?;
        commit.finish()
    }
}
