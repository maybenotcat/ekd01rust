#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryBlockDevice;

    #[derive(Clone)]
    struct RecordingBlockDevice {
        inner: MemoryBlockDevice,
        snapshots: Vec<Vec<u8>>,
    }

    impl RecordingBlockDevice {
        fn new(cfg: Config) -> Self {
            Self {
                inner: MemoryBlockDevice::new_erased(cfg).expect("memory device"),
                snapshots: Vec::new(),
            }
        }

        fn clear_snapshots(&mut self) {
            self.snapshots.clear();
        }

        fn record(&mut self) {
            self.snapshots.push(self.inner.as_bytes().to_vec());
        }
    }

    impl BlockDevice for RecordingBlockDevice {
        fn config(&self) -> Config {
            self.inner.config()
        }

        fn read(&self, block: u32, off: usize, out: &mut [u8]) -> Result<()> {
            self.inner.read(block, off, out)
        }

        fn prog(&mut self, block: u32, off: usize, data: &[u8]) -> Result<()> {
            self.inner.prog(block, off, data)?;
            self.record();
            Ok(())
        }

        fn erase(&mut self, block: u32) -> Result<()> {
            self.inner.erase(block)?;
            self.record();
            Ok(())
        }

        fn sync(&mut self) -> Result<()> {
            self.inner.sync()
        }
    }

    fn append_movestate(image: &mut [u8], cfg: Config, pair: &MetadataPair, delta: GlobalState) {
        let start = pair.active_block as usize * cfg.block_size;
        let end = start + cfg.block_size;
        let block = image.get_mut(start..end).expect("metadata block slice");
        let mut payload = Vec::new();
        payload.extend_from_slice(&delta.tag.to_le_bytes());
        payload.extend_from_slice(&delta.pair[0].to_le_bytes());
        payload.extend_from_slice(&delta.pair[1].to_le_bytes());
        let entry = CommitEntry::new(Tag::new(LFS_TYPE_MOVESTATE, 0x3ff, 12), &payload);
        let mut writer =
            MetadataCommitWriter::append(block, 16, pair.state).expect("append movestate commit");
        writer
            .write_entries(&[entry])
            .expect("write movestate entry");
        writer.finish().expect("finish movestate commit");
    }

    #[test]
    fn mutable_mount_clears_orphan_only_global_state() {
        let cfg = Config {
            block_size: 512,
            block_count: 32,
        };
        let mut builder = ImageBuilder::new(cfg).expect("builder");
        builder
            .add_inline_file("/keep.txt", b"still visible")
            .expect("create visible file");
        let mut image = builder.build().expect("build image");
        let fs = Filesystem::mount(&image, cfg).expect("mount image before orphan marker");
        let root = fs.root.clone();
        append_movestate(
            &mut image,
            cfg,
            &root,
            GlobalState {
                tag: 0x8000_0000,
                pair: [0, 0],
            },
        );
        let marked = Filesystem::mount(&image, cfg).expect("mount image with orphan marker");
        assert!(marked.global_state().has_orphans());

        let device = MemoryBlockDevice::from_bytes(cfg, &image).expect("device from image");
        let mounted = Filesystem::mount_device_mut(device).expect("mutable mount repairs marker");
        assert_eq!(
            mounted.as_filesystem().global_state(),
            GlobalState::default()
        );
        assert_eq!(
            mounted
                .read_file("/keep.txt")
                .expect("visible file survives"),
            b"still visible"
        );
    }

    #[test]
    fn cross_parent_rename_records_and_clears_move_state() {
        let cfg = Config {
            block_size: 512,
            block_count: 64,
        };
        let mut device = RecordingBlockDevice::new(cfg);
        Filesystem::format_device(&mut device).expect("format device");
        {
            let mut mounted = Filesystem::mount_device_mut(device).expect("mount device");
            mounted.create_dir("/a").expect("create source parent");
            mounted.create_dir("/b").expect("create destination parent");
            mounted
                .create_file("/a/file.txt", b"payload")
                .expect("create source file");
            device = mounted.into_device();
        }

        let before = Filesystem::mount_device(&device.inner).expect("mount before rename");
        let source_parent = before.resolve_dir("/a").expect("resolve source parent");
        let source = before
            .files_in_pair_chain(&source_parent)
            .expect("source entries")
            .into_iter()
            .find(|file| file.name == "file.txt")
            .expect("source file");
        let move_state = GlobalState {
            tag: Tag::new(LFS_TYPE_DELETE, source.id, 0).0,
            pair: source_parent.pair,
        };
        device.clear_snapshots();

        let mut mounted = Filesystem::mount_device_mut(device).expect("remount device");
        mounted
            .rename_file("/a/file.txt", "/b/file.txt")
            .expect("cross-parent rename");
        let device = mounted.into_device();

        assert!(
            device.snapshots.iter().any(|snapshot| {
                Filesystem::mount(snapshot, cfg)
                    .map(|fs| fs.global_state() == move_state)
                    .unwrap_or(false)
            }),
            "destination commit should expose a pending move state before source delete clears it"
        );
        let final_fs = Filesystem::mount_device(&device.inner).expect("mount final image");
        assert_eq!(final_fs.global_state(), GlobalState::default());
        assert_eq!(
            final_fs.read_file("/b/file.txt").expect("renamed payload"),
            b"payload"
        );
        assert!(matches!(
            final_fs.read_file("/a/file.txt"),
            Err(Error::NotFound)
        ));
    }

    #[test]
    fn mutable_mount_repairs_half_orphan_after_root_child_relocation() {
        let cfg = Config {
            block_size: 512,
            block_count: 64,
        };
        let mut device = RecordingBlockDevice::new(cfg);
        Filesystem::format_device(&mut device).expect("format device");
        {
            let mut mounted = Filesystem::mount_device_mut(device).expect("mount device");
            mounted.create_dir("/docs").expect("create child directory");
            mounted
                .create_file("/docs/note.txt", b"old")
                .expect("create file before relocation");
            device = mounted.into_device();
        }
        let (half_orphan, relocated) = {
            let mut device = device;
            let mut found = None;
            for attempt in 0..128 {
                device.clear_snapshots();
                let mut mounted = Filesystem::mount_device_mut(device)
                    .expect("remount before relocation attempt");
                mounted.set_block_cycles(Some(1));
                let data = format!("relocated-{attempt:02}").into_bytes();
                mounted
                    .write_file("/docs/note.txt", &data)
                    .expect("root child pair relocation attempt");
                device = mounted.into_device();

                // With C's exact `block_cycles` cadence, `block_cycles = 1`
                // relocates only when `(rev + 1) % 3 == 0`. Earlier attempts
                // are still real metadata writes that may append or compact,
                // so the test loops until it sees the half-orphan parent
                // update snapshot that only relocation can produce.
                if let Some(snapshot) = device.snapshots.iter().find(|snapshot| {
                    Filesystem::mount(snapshot, cfg)
                        .map(|fs| {
                            fs.global_state().has_orphans()
                                && fs.read_file("/docs/note.txt").is_ok_and(|read| read == data)
                        })
                        .unwrap_or(false)
                }) {
                    found = Some((snapshot.clone(), data));
                    break;
                }
            }
            found.expect("parent update snapshot should expose a pending half-orphan")
        };

        let repair_device =
            MemoryBlockDevice::from_bytes(cfg, &half_orphan).expect("half-orphan snapshot device");
        let repaired =
            Filesystem::mount_device_mut(repair_device).expect("mutable mount repairs half-orphan");
        assert_eq!(
            repaired.as_filesystem().global_state(),
            GlobalState::default(),
            "half-orphan repair should clear the orphan count"
        );
        assert_eq!(
            repaired
                .read_file("/docs/note.txt")
                .expect("relocated file remains visible after repair"),
            relocated
        );

        let docs = repaired
            .as_filesystem()
            .resolve_dir("/docs")
            .expect("resolve relocated child directory");
        let predecessor = repaired
            .predecessor_in_thread(docs.pair)
            .expect("scan metadata thread")
            .expect("relocated child directory stays threaded");
        let tail = predecessor
            .tail()
            .expect("read predecessor tail")
            .expect("predecessor has a tail");
        assert!(
            !tail.split && FilesystemMut::<MemoryBlockDevice>::pair_is_sync(tail.pair, docs.pair),
            "repair should retarget the softtail to the relocated pair"
        );
    }

    #[test]
    fn block_cycles_one_uses_c_odd_modulus_before_relocating() {
        let cfg = Config {
            block_size: 512,
            block_count: 64,
        };
        let mut device = RecordingBlockDevice::new(cfg);
        Filesystem::format_device(&mut device).expect("format device");
        let mut mounted = Filesystem::mount_device_mut(device).expect("mount device");
        mounted.create_dir("/docs").expect("create child directory");
        mounted
            .create_file("/docs/note.txt", b"old")
            .expect("create file before relocation check");
        let before = mounted
            .as_filesystem()
            .resolve_dir("/docs")
            .expect("resolve child before block-cycle write")
            .pair;

        mounted.set_block_cycles(Some(1));
        mounted
            .write_file("/docs/note.txt", b"first update")
            .expect("first update should stay in place");
        let after = mounted
            .as_filesystem()
            .resolve_dir("/docs")
            .expect("resolve child after block-cycle write")
            .pair;

        // In upstream C, block_cycles=1 maps to an odd interval of 3, so a
        // freshly created child pair at rev=1 must not relocate on the next
        // write. A simple `rev >= block_cycles` policy would move here and
        // would fail this test by changing the DIRSTRUCT pair.
        assert_eq!(after, before);
    }
}
