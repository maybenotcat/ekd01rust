use alloc::{collections::BTreeMap, string::String, vec::Vec};

use crate::{
    commit::CommitState,
    format::{
        Entry, LFS_NULL, LFS_TYPE_CREATE, LFS_TYPE_CTZSTRUCT, LFS_TYPE_DELETE, LFS_TYPE_DIR,
        LFS_TYPE_DIRSTRUCT, LFS_TYPE_FCRC, LFS_TYPE_HARDTAIL, LFS_TYPE_INLINESTRUCT,
        LFS_TYPE_MOVESTATE, LFS_TYPE_REG, LFS_TYPE_SOFTTAIL, LFS_TYPE_USERATTR, Tag, be32, crc32,
        le32, seq_after,
    },
    types::{DirEntry, Error, FileType, Result},
};

/// A metadata pair is littlefs's atomic metadata unit.
///
/// Each pair is stored in two blocks. On mount we parse both blocks, discard
/// corrupt/incomplete logs, and keep the one with the newest revision using
/// sequence arithmetic. Higher-level directory code follows `hardtail` links
/// when a directory has been split across multiple metadata pairs.
#[derive(Debug, Clone)]
pub(crate) struct MetadataPair {
    pub(crate) pair: [u32; 2],
    pub(crate) active_block: u32,
    pub(crate) rev: u32,
    pub(crate) state: CommitState,
    data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MetadataTail {
    /// C littlefs stores both softtails and hardtails as `LFS_TYPE_TAIL +
    /// split`. A hardtail (`split == true`) extends the current directory's
    /// sorted entry chain, while a softtail (`split == false`) only links this
    /// metadata pair into the filesystem-wide threaded list used by orphan
    /// repair and allocation scans.
    pub(crate) split: bool,
    pub(crate) pair: [u32; 2],
}

impl MetadataPair {
    pub(crate) fn read(image: &[u8], cfg: crate::types::Config, pair: [u32; 2]) -> Result<Self> {
        // Power loss may leave one side of the pair with an incomplete commit.
        // Treat the two blocks independently and accept either valid side.
        let a = MetadataBlock::read(image, cfg, pair[0]);
        let b = MetadataBlock::read(image, cfg, pair[1]);

        match (a, b) {
            (Ok(a), Ok(b)) => {
                if seq_after(a.rev, b.rev) {
                    Ok(Self {
                        pair,
                        active_block: pair[0],
                        rev: a.rev,
                        state: a.state,
                        data: a.data,
                    })
                } else {
                    Ok(Self {
                        pair,
                        active_block: pair[1],
                        rev: b.rev,
                        state: b.state,
                        data: b.data,
                    })
                }
            }
            (Ok(a), Err(_)) => Ok(Self {
                pair,
                active_block: pair[0],
                rev: a.rev,
                state: a.state,
                data: a.data,
            }),
            (Err(_), Ok(b)) => Ok(Self {
                pair,
                active_block: pair[1],
                rev: b.rev,
                state: b.state,
                data: b.data,
            }),
            (Err(e), Err(_)) => Err(e),
        }
    }

    pub(crate) fn read_from<F>(
        cfg: crate::types::Config,
        pair: [u32; 2],
        mut read_block: F,
    ) -> Result<Self>
    where
        F: FnMut(u32, &mut [u8]) -> Result<()>,
    {
        // This is the block-device twin of `read`. It intentionally preserves
        // the exact "pick newest valid side" logic while avoiding the old
        // requirement that mount first materialize a contiguous full image.
        let a = MetadataBlock::read_from(cfg, pair[0], &mut read_block);
        let b = MetadataBlock::read_from(cfg, pair[1], &mut read_block);

        match (a, b) {
            (Ok(a), Ok(b)) => {
                if seq_after(a.rev, b.rev) {
                    Ok(Self {
                        pair,
                        active_block: pair[0],
                        rev: a.rev,
                        state: a.state,
                        data: a.data,
                    })
                } else {
                    Ok(Self {
                        pair,
                        active_block: pair[1],
                        rev: b.rev,
                        state: b.state,
                        data: b.data,
                    })
                }
            }
            (Ok(a), Err(_)) => Ok(Self {
                pair,
                active_block: pair[0],
                rev: a.rev,
                state: a.state,
                data: a.data,
            }),
            (Err(_), Ok(b)) => Ok(Self {
                pair,
                active_block: pair[1],
                rev: b.rev,
                state: b.state,
                data: b.data,
            }),
            (Err(e), Err(_)) => Err(e),
        }
    }

    pub(crate) fn find(&self, want: Tag) -> Result<Entry> {
        let mask = want.lookup_mask();
        let mut found = None;
        // Metadata logs are append-only. Scanning forward and replacing the
        // previous match gives the same result as the old reverse search, but
        // does not require `MetadataPair` to keep a permanent `Vec<Entry>`.
        self.visit_entries(|tag, data| {
            if tag.matches(mask, want) {
                found = if tag.is_delete() {
                    None
                } else {
                    Some(Entry {
                        tag,
                        data: data.to_vec(),
                    })
                };
            }
            Ok(())
        })?;
        found.ok_or(Error::NotFound)
    }

    pub(crate) fn visit_entries<'a, F>(&'a self, mut visitor: F) -> Result<()>
    where
        F: FnMut(Tag, &'a [u8]) -> Result<()>,
    {
        // Re-decode the active block's committed prefix on demand. This is the
        // key memory pivot away from the previous owned AST: the long-lived
        // `MetadataPair` stores only the block bytes and commit cursor, while
        // callers decide whether to stream, lookup, or materialize entries.
        let data = &self.data;
        let mut crc = crc32(0xffff_ffff, data.get(0..4).ok_or(Error::Corrupt)?);
        let mut off = 4usize;
        let mut ptag = Tag(0xffff_ffff);

        while off < self.state.off && off + 4 <= data.len() {
            let disk_tag = be32(&data[off..off + 4])?;
            let tag = Tag((ptag.0 ^ disk_tag) & 0x7fff_ffff);
            if !tag.is_valid() || tag.0 == 0 || tag.0 == LFS_NULL {
                break;
            }
            let dsize = tag.dsize()?;
            let end = off.checked_add(dsize).ok_or(Error::Corrupt)?;
            if end > data.len() || end > self.state.off {
                break;
            }

            crc = crc32(crc, &data[off..off + 4]);
            let entry_data = &data[off + 4..end];
            off = end;
            ptag = tag;

            if tag.is_ccrc() {
                if entry_data.len() < 4 {
                    break;
                }
                let dcrc = le32(&entry_data[0..4])?;
                if crc != dcrc {
                    break;
                }
                let valid_state = (tag.type3() & 1) as u32;
                ptag = Tag(ptag.0 ^ (valid_state << 31));
                crc = 0xffff_ffff;
            } else {
                if tag.type3() != LFS_TYPE_FCRC {
                    visitor(tag, entry_data)?;
                }
                crc = crc32(crc, entry_data);
            }
        }

        Ok(())
    }

    pub(crate) fn fold_commit_crcs_into_seed(&self, seed: &mut u32) -> Result<()> {
        // The mounted allocator needs C's allocation seed, which is the CRC32
        // fold of every valid commit CRC in visible metadata. Earlier versions
        // retained a `Vec<u32>` of those CRCs in every `MetadataPair`; reusing
        // the committed metadata prefix avoids that permanent allocation while
        // preserving the exact fold order.
        let data = &self.data;
        let mut crc = crc32(0xffff_ffff, data.get(0..4).ok_or(Error::Corrupt)?);
        let mut off = 4usize;
        let mut ptag = Tag(0xffff_ffff);

        while off < self.state.off {
            if off + 4 > data.len() {
                return Err(Error::Corrupt);
            }
            let disk_tag = be32(&data[off..off + 4])?;
            let tag = Tag((ptag.0 ^ disk_tag) & 0x7fff_ffff);
            if !tag.is_valid() || tag.0 == 0 || tag.0 == LFS_NULL {
                return Err(Error::Corrupt);
            }
            let dsize = tag.dsize()?;
            let end = off.checked_add(dsize).ok_or(Error::Corrupt)?;
            if end > data.len() || end > self.state.off {
                return Err(Error::Corrupt);
            }

            crc = crc32(crc, &data[off..off + 4]);
            let entry_data = &data[off + 4..end];
            off = end;
            ptag = tag;

            if tag.is_ccrc() {
                if entry_data.len() < 4 {
                    return Err(Error::Corrupt);
                }
                let dcrc = le32(&entry_data[0..4])?;
                if crc != dcrc {
                    return Err(Error::Corrupt);
                }
                *seed = crc32(*seed, &crc.to_le_bytes());
                let valid_state = (tag.type3() & 1) as u32;
                ptag = Tag(ptag.0 ^ (valid_state << 31));
                crc = 0xffff_ffff;
            } else {
                crc = crc32(crc, entry_data);
            }
        }

        Ok(())
    }

    pub(crate) fn files(&self) -> Result<Vec<FileRecord>> {
        let mut out = Vec::new();
        self.fold_dir(|_id, file| {
            out.push(file);
            Ok(())
        })?;
        Ok(out)
    }

    pub(crate) fn find_name_no_attrs(&self, name: &str) -> Result<Option<FileRecord>> {
        self.find_name_borrowed(name, AttrSelection::None)
    }

    pub(crate) fn find_dir_entry(&self, name: &str) -> Result<Option<DirEntry>> {
        let mut found = None;
        self.fold_borrowed_dir(AttrSelection::None, |_id, file| {
            if file.name == name.as_bytes() {
                found = Some(file.dir_entry()?);
            }
            Ok(())
        })?;
        Ok(found)
    }

    pub(crate) fn find_attr(&self, name: &str, attr_type: u8) -> Result<Option<Vec<u8>>> {
        let mut found = None;
        self.fold_borrowed_dir(AttrSelection::One(attr_type), |_id, file| {
            if file.name == name.as_bytes() {
                found = file.attrs.get(&attr_type).map(|attr| attr.to_vec());
            }
            Ok(())
        })?;
        Ok(found)
    }

    pub(crate) fn copy_attr_into(
        &self,
        name: &str,
        attr_type: u8,
        out: &mut [u8],
    ) -> Result<Option<usize>> {
        let mut found = None;
        self.fold_borrowed_dir(AttrSelection::One(attr_type), |_id, file| {
            if file.name == name.as_bytes()
                && let Some(attr) = file.attrs.get(&attr_type)
            {
                if out.len() < attr.len() {
                    return Err(Error::NoSpace);
                }
                out[..attr.len()].copy_from_slice(attr);
                found = Some(attr.len());
            }
            Ok(())
        })?;
        Ok(found)
    }

    pub(crate) fn storage_refs(&self) -> Result<Vec<StorageRef>> {
        let mut refs = Vec::new();
        self.fold_borrowed_dir(AttrSelection::None, |_id, file| {
            match (file.ty.clone(), file.data) {
                (FileType::Dir, BorrowedFileData::Directory(pair)) => {
                    refs.push(StorageRef::Directory(pair));
                }
                (FileType::File, BorrowedFileData::Ctz { head, size }) => {
                    refs.push(StorageRef::Ctz { head, size });
                }
                _ => {}
            }
            Ok(())
        })?;
        Ok(refs)
    }

    fn find_name_borrowed(&self, name: &str, attrs: AttrSelection) -> Result<Option<FileRecord>> {
        let mut found = None;
        self.fold_borrowed_dir(attrs, |_id, file| {
            if file.name == name.as_bytes() {
                found = Some(file.to_owned_record()?);
            }
            Ok(())
        })?;
        Ok(found)
    }

    pub(crate) fn file_count(&self) -> Result<usize> {
        let mut count = 0;
        self.fold_dir(|_id, _file| {
            count += 1;
            Ok(())
        })?;
        Ok(count)
    }

    pub(crate) fn fold_dir<F>(&self, mut visitor: F) -> Result<()>
    where
        F: FnMut(u16, FileRecord) -> Result<()>,
    {
        // This is the directory-level streaming surface. It deliberately keeps
        // `Vec<FileRecord>` out of the core parser and lets high-level callers
        // choose whether to lookup one name, count entries, or collect a public
        // listing. The temporary id map is bounded by one metadata pair's log,
        // not the whole filesystem image.
        for (id, rec) in self.file_builders()? {
            if let Some(file) = FileRecord::from_builder(id, rec) {
                visitor(id, file)?;
            }
        }
        Ok(())
    }

    fn fold_borrowed_dir<'a, F>(&'a self, attrs: AttrSelection, mut visitor: F) -> Result<()>
    where
        F: FnMut(u16, BorrowedFileRecord<'a>) -> Result<()>,
    {
        for (id, rec) in self.borrowed_file_builders(attrs)? {
            if let Some(file) = BorrowedFileRecord::from_builder(id, rec)? {
                visitor(id, file)?;
            }
        }
        Ok(())
    }

    fn file_builders(&self) -> Result<BTreeMap<u16, FileRecordBuilder>> {
        // This folds a committed metadata log into directory entries. The
        // create/delete tags model insertion/deletion in an imaginary sorted
        // id array, so ids after the affected position must shift.
        let mut by_id = BTreeMap::<u16, FileRecordBuilder>::new();
        self.visit_entries(|tag, data| {
            if tag.type3() == LFS_TYPE_CREATE {
                shift_create(&mut by_id, tag.id());
                return Ok(());
            }
            if tag.type3() == LFS_TYPE_DELETE {
                by_id.remove(&tag.id());
                shift_delete(&mut by_id, tag.id());
                return Ok(());
            }
            if tag.id() == 0x3ff {
                return Ok(());
            }

            let rec = by_id.entry(tag.id()).or_default();
            match tag.type3() {
                LFS_TYPE_REG => {
                    // NAME tags use the low type values for file kind. The
                    // payload is the file name, not file contents.
                    rec.ty = Some(FileType::File);
                    rec.name = Some(string_from_ascii(data)?);
                }
                LFS_TYPE_DIR => {
                    rec.ty = Some(FileType::Dir);
                    rec.name = Some(string_from_ascii(data)?);
                }
                LFS_TYPE_INLINESTRUCT => {
                    // Small files live directly in metadata. This is why
                    // reading the metadata pair is enough to read many files.
                    rec.data = Some(FileData::Inline(data.to_vec()));
                }
                LFS_TYPE_CTZSTRUCT => {
                    // Larger files are represented by the CTZ head block plus
                    // total file size. The actual block walking is in fs.rs.
                    if data.len() != 8 {
                        return Err(Error::Corrupt);
                    }
                    rec.data = Some(FileData::Ctz {
                        head: le32(&data[0..4])?,
                        size: le32(&data[4..8])?,
                    });
                }
                LFS_TYPE_DIRSTRUCT => {
                    // Directories point at another metadata pair. That pair may
                    // itself have hardtail links if the directory was split.
                    if data.len() != 8 {
                        return Err(Error::Corrupt);
                    }
                    rec.data = Some(FileData::Directory([
                        le32(&data[0..4])?,
                        le32(&data[4..8])?,
                    ]));
                }
                ty if (LFS_TYPE_USERATTR..LFS_TYPE_CREATE).contains(&ty) => {
                    // User attributes are type 0x300 + user-supplied attr id.
                    // They supersede by id just like file structs.
                    let attr_type = (ty - LFS_TYPE_USERATTR) as u8;
                    if tag.is_delete() {
                        rec.attrs.remove(&attr_type);
                    } else {
                        rec.attrs.insert(attr_type, data.to_vec());
                    }
                }
                _ => {}
            }
            Ok(())
        })?;

        Ok(by_id)
    }

    fn borrowed_file_builders<'a>(
        &'a self,
        attrs: AttrSelection,
    ) -> Result<BTreeMap<u16, BorrowedFileRecordBuilder<'a>>> {
        let mut by_id = BTreeMap::<u16, BorrowedFileRecordBuilder<'a>>::new();
        self.visit_entries(|tag, data| {
            if tag.type3() == LFS_TYPE_CREATE {
                shift_create_borrowed(&mut by_id, tag.id());
                return Ok(());
            }
            if tag.type3() == LFS_TYPE_DELETE {
                by_id.remove(&tag.id());
                shift_delete_borrowed(&mut by_id, tag.id());
                return Ok(());
            }
            if tag.id() == 0x3ff {
                return Ok(());
            }

            let rec = by_id.entry(tag.id()).or_default();
            match tag.type3() {
                LFS_TYPE_REG => {
                    validate_ascii(data)?;
                    rec.ty = Some(FileType::File);
                    rec.name = Some(data);
                }
                LFS_TYPE_DIR => {
                    validate_ascii(data)?;
                    rec.ty = Some(FileType::Dir);
                    rec.name = Some(data);
                }
                LFS_TYPE_INLINESTRUCT => {
                    rec.data = Some(BorrowedFileData::Inline(data));
                }
                LFS_TYPE_CTZSTRUCT => {
                    if data.len() != 8 {
                        return Err(Error::Corrupt);
                    }
                    rec.data = Some(BorrowedFileData::Ctz {
                        head: le32(&data[0..4])?,
                        size: le32(&data[4..8])?,
                    });
                }
                LFS_TYPE_DIRSTRUCT => {
                    if data.len() != 8 {
                        return Err(Error::Corrupt);
                    }
                    rec.data = Some(BorrowedFileData::Directory([
                        le32(&data[0..4])?,
                        le32(&data[4..8])?,
                    ]));
                }
                ty if (LFS_TYPE_USERATTR..LFS_TYPE_CREATE).contains(&ty) => {
                    let attr_type = (ty - LFS_TYPE_USERATTR) as u8;
                    if !attrs.includes(attr_type) {
                        return Ok(());
                    }
                    if tag.is_delete() {
                        rec.attrs.remove(&attr_type);
                    } else {
                        rec.attrs.insert(attr_type, data);
                    }
                }
                _ => {}
            }
            Ok(())
        })?;

        Ok(by_id)
    }

    pub(crate) fn hardtail(&self) -> Result<Option<[u32; 2]>> {
        // Directory listing must only follow hardtails. Softtails are equally
        // real on disk, but they belong to the global metadata-pair thread and
        // would make child directories appear as entries in their parent if we
        // followed them here.
        Ok(self
            .tail()?
            .and_then(|tail| tail.split.then_some(tail.pair)))
    }

    pub(crate) fn tail(&self) -> Result<Option<MetadataTail>> {
        // Tail tags are unique id-0x3ff records. As with other unique tags, the
        // newest tail in the log wins. Supporting both variants is the small
        // parser hook that lets higher layers reason about littlefs' threaded
        // metadata list without disturbing normal directory traversal.
        let mut found = None;
        self.visit_entries(|tag, data| {
            let split = match tag.type3() {
                LFS_TYPE_SOFTTAIL => false,
                LFS_TYPE_HARDTAIL => true,
                _ => return Ok(()),
            };
            if data.len() != 8 {
                return Err(Error::Corrupt);
            }
            found = Some(MetadataTail {
                split,
                pair: [le32(&data[0..4])?, le32(&data[4..8])?],
            });
            Ok(())
        })?;
        Ok(found)
    }

    pub(crate) fn global_state_delta(&self) -> Result<GlobalState> {
        let mut found = GlobalState::default();
        self.visit_entries(|tag, data| {
            if tag.type3() != LFS_TYPE_MOVESTATE {
                return Ok(());
            }
            if data.len() != 12 {
                return Err(Error::Corrupt);
            }
            found = GlobalState {
                tag: le32(&data[0..4])?,
                pair: [le32(&data[4..8])?, le32(&data[8..12])?],
            };
            Ok(())
        })?;
        Ok(found)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct GlobalState {
    pub(crate) tag: u32,
    pub(crate) pair: [u32; 2],
}

impl GlobalState {
    pub(crate) fn xor(&mut self, other: GlobalState) {
        self.tag ^= other.tag;
        self.pair[0] ^= other.pair[0];
        self.pair[1] ^= other.pair[1];
    }

    pub(crate) fn is_zero(self) -> bool {
        self.tag == 0 && self.pair == [0, 0]
    }

    pub(crate) fn orphan_count(count: u16) -> Result<Self> {
        Self::default().with_orphan_count(count)
    }

    pub(crate) fn with_orphan_count(mut self, count: u16) -> Result<Self> {
        if count > 0x1ff {
            return Err(Error::Unsupported);
        }
        // `lfs_fs_preporphans` keeps the low size bits as the orphan count and
        // mirrors the non-zero state into the high invalid bit. The remaining
        // tag bits can still carry a pending move, so this helper deliberately
        // edits only the orphan-related bits.
        self.tag = (self.tag & !0x8000_01ff) | u32::from(count);
        if count != 0 {
            self.tag |= 0x8000_0000;
        }
        Ok(self)
    }

    pub(crate) fn has_move(self) -> bool {
        self.tag & 0x7000_0000 != 0
    }

    pub(crate) fn has_orphans(self) -> bool {
        self.tag & 0x8000_0000 != 0 || self.tag & 0x0000_01ff != 0
    }

    pub(crate) fn move_delta(self) -> Result<Option<GlobalState>> {
        if !self.has_move() {
            return Ok(None);
        }
        let move_tag = Tag(self.tag);
        if move_tag.type3() != LFS_TYPE_DELETE || self.pair == [0, 0] {
            return Err(Error::Unsupported);
        }
        Ok(Some(GlobalState {
            tag: Tag::new(LFS_TYPE_DELETE, move_tag.id(), 0).0,
            pair: self.pair,
        }))
    }
}

#[derive(Debug)]
struct MetadataBlock {
    rev: u32,
    state: CommitState,
    data: Vec<u8>,
}

impl MetadataBlock {
    fn read(image: &[u8], cfg: crate::types::Config, block: u32) -> Result<Self> {
        let block = block as usize;
        if block >= cfg.block_count {
            return Err(Error::OutOfBounds);
        }
        let start = block * cfg.block_size;
        let data = image
            .get(start..start + cfg.block_size)
            .ok_or(Error::OutOfBounds)?;
        Self::parse(data.to_vec())
    }

    fn read_from<F>(cfg: crate::types::Config, block: u32, read_block: &mut F) -> Result<Self>
    where
        F: FnMut(u32, &mut [u8]) -> Result<()>,
    {
        if block as usize >= cfg.block_count {
            return Err(Error::OutOfBounds);
        }
        let mut data = alloc::vec![0xff; cfg.block_size];
        read_block(block, &mut data)?;
        Self::parse(data)
    }

    fn parse(data: Vec<u8>) -> Result<Self> {
        let rev = le32(&data[0..4])?;
        if rev == LFS_NULL {
            return Err(Error::Corrupt);
        }

        let mut crc = crc32(0xffff_ffff, &data[0..4]);
        let mut off = 4;
        let mut ptag = Tag(0xffff_ffff);
        let mut committed_state = None;
        let mut committed_any = false;

        while off + 4 <= data.len() {
            // Tags are stored as big-endian XOR deltas. `ptag` is the previous
            // decoded tag, starting from all ones. The valid bit is masked out
            // after decoding, matching littlefs's fetch logic.
            let disk_tag = be32(&data[off..off + 4])?;
            let tag = Tag((ptag.0 ^ disk_tag) & 0x7fff_ffff);
            if !tag.is_valid() || tag.0 == 0 || tag.0 == LFS_NULL {
                break;
            }
            let dsize = tag.dsize()?;
            if off + dsize > data.len() {
                break;
            }

            // The C implementation CRCs the raw big-endian tag word first.
            // For CRC tags, only the tag word participates before comparing
            // against the stored little-endian CRC value.
            crc = crc32(crc, &data[off..off + 4]);
            let entry_data = &data[off + 4..off + dsize];

            off += dsize;
            ptag = tag;

            if tag.is_ccrc() {
                if entry_data.len() < 4 {
                    break;
                }
                let dcrc = le32(&entry_data[0..4])?;
                if crc == dcrc {
                    // A valid CRC commits every entry seen since the previous
                    // CRC. Later semantic scans re-decode the committed prefix
                    // from `data`, avoiding a long-lived `Vec<Entry>` AST.
                    committed_any = true;
                    let valid_state = (tag.type3() & 1) as u32;
                    ptag = Tag(ptag.0 ^ (valid_state << 31));
                    committed_state = Some(CommitState { off, ptag: ptag.0 });
                } else {
                    break;
                }
                crc = 0xffff_ffff;
            } else {
                // Non-CRC entries include their full payload in the running
                // commit CRC.
                crc = crc32(crc, &entry_data);
            }
        }

        if !committed_any {
            return Err(Error::Corrupt);
        }
        let state = committed_state.ok_or(Error::Corrupt)?;
        let mut data = data;
        data.truncate(state.off);
        data.shrink_to_fit();
        Ok(Self { rev, state, data })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FileRecord {
    pub(crate) id: u16,
    pub(crate) name: String,
    pub(crate) ty: FileType,
    pub(crate) data: FileData,
    pub(crate) attrs: BTreeMap<u8, Vec<u8>>,
}

impl FileRecord {
    fn from_builder(id: u16, rec: FileRecordBuilder) -> Option<Self> {
        let (Some(name), Some(ty)) = (rec.name, rec.ty) else {
            return None;
        };
        let data = rec.data.unwrap_or_else(|| FileData::Inline(Vec::new()));
        Some(Self {
            id,
            name,
            ty,
            data,
            attrs: rec.attrs,
        })
    }

    /// Converts the internal semantic record into the public `DirEntry` shape.
    /// Directory sizes are reported as zero to match littlefs's public stat
    /// behavior.
    pub(crate) fn dir_entry(&self) -> DirEntry {
        let size = match &self.data {
            FileData::Inline(data) => data.len() as u32,
            FileData::Ctz { size, .. } => *size,
            FileData::Directory(_) => 0,
        };
        DirEntry {
            name: self.name.clone(),
            ty: self.ty.clone(),
            size,
        }
    }
}

#[derive(Debug, Default, Clone)]
struct FileRecordBuilder {
    name: Option<String>,
    ty: Option<FileType>,
    data: Option<FileData>,
    attrs: BTreeMap<u8, Vec<u8>>,
}

#[derive(Debug, Clone, Copy)]
enum AttrSelection {
    None,
    One(u8),
}

impl AttrSelection {
    fn includes(self, attr_type: u8) -> bool {
        match self {
            Self::None => false,
            Self::One(want) => want == attr_type,
        }
    }
}

#[derive(Debug, Default, Clone)]
struct BorrowedFileRecordBuilder<'a> {
    name: Option<&'a [u8]>,
    ty: Option<FileType>,
    data: Option<BorrowedFileData<'a>>,
    attrs: BTreeMap<u8, &'a [u8]>,
}

#[derive(Debug, Clone)]
struct BorrowedFileRecord<'a> {
    id: u16,
    name: &'a [u8],
    ty: FileType,
    data: BorrowedFileData<'a>,
    attrs: BTreeMap<u8, &'a [u8]>,
}

impl<'a> BorrowedFileRecord<'a> {
    fn from_builder(id: u16, rec: BorrowedFileRecordBuilder<'a>) -> Result<Option<Self>> {
        let (Some(name), Some(ty)) = (rec.name, rec.ty) else {
            return Ok(None);
        };
        validate_ascii(name)?;
        let data = rec.data.unwrap_or(BorrowedFileData::Inline(&[]));
        Ok(Some(Self {
            id,
            name,
            ty,
            data,
            attrs: rec.attrs,
        }))
    }

    fn to_owned_record(&self) -> Result<FileRecord> {
        Ok(FileRecord {
            id: self.id,
            name: string_from_ascii(self.name)?,
            ty: self.ty.clone(),
            data: self.data.to_owned(),
            attrs: self
                .attrs
                .iter()
                .map(|(attr_type, data)| (*attr_type, data.to_vec()))
                .collect(),
        })
    }

    fn dir_entry(&self) -> Result<DirEntry> {
        Ok(DirEntry {
            name: string_from_ascii(self.name)?,
            ty: self.ty.clone(),
            size: self.data.size_for_dir_entry(),
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) enum FileData {
    Inline(Vec<u8>),
    Ctz { head: u32, size: u32 },
    Directory([u32; 2]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StorageRef {
    Ctz { head: u32, size: u32 },
    Directory([u32; 2]),
}

#[derive(Debug, Clone, Copy)]
enum BorrowedFileData<'a> {
    Inline(&'a [u8]),
    Ctz { head: u32, size: u32 },
    Directory([u32; 2]),
}

impl BorrowedFileData<'_> {
    fn to_owned(self) -> FileData {
        match self {
            Self::Inline(data) => FileData::Inline(data.to_vec()),
            Self::Ctz { head, size } => FileData::Ctz { head, size },
            Self::Directory(pair) => FileData::Directory(pair),
        }
    }

    fn size_for_dir_entry(self) -> u32 {
        match self {
            Self::Inline(data) => data.len() as u32,
            Self::Ctz { size, .. } => size,
            Self::Directory(_) => 0,
        }
    }
}

fn shift_create(map: &mut BTreeMap<u16, FileRecordBuilder>, id: u16) {
    // CREATE inserts a new id position. Existing ids at or after that position
    // move up by one.
    let keys: Vec<_> = map.keys().copied().filter(|key| *key >= id).collect();
    for key in keys.into_iter().rev() {
        if let Some(value) = map.remove(&key) {
            map.insert(key + 1, value);
        }
    }
}

fn shift_create_borrowed<'a>(map: &mut BTreeMap<u16, BorrowedFileRecordBuilder<'a>>, id: u16) {
    let keys: Vec<_> = map.keys().copied().filter(|key| *key >= id).collect();
    for key in keys.into_iter().rev() {
        if let Some(value) = map.remove(&key) {
            map.insert(key + 1, value);
        }
    }
}

fn shift_delete(map: &mut BTreeMap<u16, FileRecordBuilder>, id: u16) {
    // DELETE removes an id position. Later ids move down by one.
    let keys: Vec<_> = map.keys().copied().filter(|key| *key > id).collect();
    for key in keys {
        if let Some(value) = map.remove(&key) {
            map.insert(key - 1, value);
        }
    }
}

fn shift_delete_borrowed<'a>(map: &mut BTreeMap<u16, BorrowedFileRecordBuilder<'a>>, id: u16) {
    let keys: Vec<_> = map.keys().copied().filter(|key| *key > id).collect();
    for key in keys {
        if let Some(value) = map.remove(&key) {
            map.insert(key - 1, value);
        }
    }
}

fn validate_ascii(data: &[u8]) -> Result<()> {
    if data.iter().any(|b| *b == 0 || *b > 0x7f) {
        return Err(Error::Utf8);
    }
    Ok(())
}

fn string_from_ascii(data: &[u8]) -> Result<String> {
    validate_ascii(data)?;
    String::from_utf8(data.to_vec()).map_err(|_| Error::Utf8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commit::{CommitEntry, MetadataCommitWriter};

    fn pair_with_tags(tags: Vec<Entry>) -> MetadataPair {
        let mut block = vec![0xff; 128];
        let mut writer = MetadataCommitWriter::new(&mut block, 16).expect("metadata writer");
        writer.write_revision(1).expect("write revision");
        let entries = tags
            .iter()
            .map(|entry| CommitEntry::new(entry.tag, &entry.data))
            .collect::<Vec<_>>();
        writer.write_entries(&entries).expect("write entries");
        writer.finish().expect("finish metadata commit");

        let parsed = MetadataBlock::parse(block).expect("parse synthetic pair");
        MetadataPair {
            pair: [0, 1],
            active_block: 0,
            rev: parsed.rev,
            state: parsed.state,
            data: parsed.data,
        }
    }

    fn movestate_entry(tag: u32, pair: [u32; 2]) -> Entry {
        let mut data = Vec::new();
        data.extend_from_slice(&tag.to_le_bytes());
        data.extend_from_slice(&pair[0].to_le_bytes());
        data.extend_from_slice(&pair[1].to_le_bytes());
        Entry {
            tag: Tag::new(LFS_TYPE_MOVESTATE, 0x3ff, 12),
            data,
        }
    }

    #[test]
    fn global_state_delta_uses_latest_movestate_entry() {
        let pair = pair_with_tags(vec![
            movestate_entry(0x1234_0001, [10, 11]),
            movestate_entry(0x0000_0001, [3, 7]),
        ]);

        assert_eq!(
            pair.global_state_delta().expect("fold movestate entries"),
            GlobalState {
                tag: 0x0000_0001,
                pair: [3, 7],
            }
        );
    }

    #[test]
    fn global_state_delta_rejects_malformed_movestate_payload() {
        let pair = pair_with_tags(vec![Entry {
            tag: Tag::new(LFS_TYPE_MOVESTATE, 0x3ff, 4),
            data: vec![0, 1, 2, 3],
        }]);

        assert!(matches!(pair.global_state_delta(), Err(Error::Corrupt)));
    }

    fn create_entry(id: u16) -> Entry {
        Entry {
            tag: Tag::new(LFS_TYPE_CREATE, id, 0),
            data: Vec::new(),
        }
    }

    fn delete_entry(id: u16) -> Entry {
        Entry {
            tag: Tag::new(LFS_TYPE_DELETE, id, 0),
            data: Vec::new(),
        }
    }

    fn name_entry(id: u16, name: &str) -> Entry {
        Entry {
            tag: Tag::new(LFS_TYPE_REG, id, name.len() as u16),
            data: name.as_bytes().to_vec(),
        }
    }

    fn inline_entry(id: u16, data: &[u8]) -> Entry {
        Entry {
            tag: Tag::new(LFS_TYPE_INLINESTRUCT, id, data.len() as u16),
            data: data.to_vec(),
        }
    }

    #[test]
    fn find_name_folds_directory_ids_without_materializing_files_first() {
        // The delete of id 0 shifts beta from id 1 to id 0. This pins the
        // splice behavior that lookup, count, and high-level collection share
        // through `fold_dir`.
        let pair = pair_with_tags(vec![
            create_entry(0),
            name_entry(0, "alpha"),
            inline_entry(0, b"a"),
            create_entry(1),
            name_entry(1, "beta"),
            inline_entry(1, b"b"),
            delete_entry(0),
        ]);

        assert!(
            pair.find_name_no_attrs("alpha")
                .expect("lookup alpha")
                .is_none()
        );
        let beta = pair
            .find_name_no_attrs("beta")
            .expect("lookup beta")
            .expect("beta survives");
        assert_eq!(beta.id, 0);
        assert_eq!(beta.dir_entry().size, 1);
        assert_eq!(pair.file_count().expect("count files"), 1);
    }

    #[test]
    fn global_state_classifies_move_and_orphan_bits_separately() {
        let move_only = GlobalState {
            tag: Tag::new(LFS_TYPE_DELETE, 5, 0).0,
            pair: [12, 13],
        };
        assert!(move_only.has_move());
        assert!(!move_only.has_orphans());
        assert_eq!(move_only.move_delta().expect("move delta"), Some(move_only));

        let orphan_only = GlobalState {
            tag: 0x8000_0000,
            pair: [0, 0],
        };
        assert!(!orphan_only.has_move());
        assert!(orphan_only.has_orphans());
        assert_eq!(orphan_only.move_delta().expect("no move"), None);

        let combined = GlobalState {
            tag: 0x8000_0000 | Tag::new(LFS_TYPE_DELETE, 7, 0).0,
            pair: [20, 21],
        };
        assert!(combined.has_move());
        assert!(combined.has_orphans());
        assert_eq!(
            combined.move_delta().expect("combined move delta"),
            Some(GlobalState {
                tag: Tag::new(LFS_TYPE_DELETE, 7, 0).0,
                pair: [20, 21],
            })
        );
    }
}
