use super::*;

impl<'fs, D: BlockDevice + 'static> FileWriter<'fs, D> {
    pub fn write(&mut self, data: &[u8]) -> Result<usize> {
        if self.stream.is_some() {
            self.write_streaming(data)?;
            return Ok(data.len());
        }

        let end = self.pos.checked_add(data.len()).ok_or(Error::NoSpace)?;
        if end > self.fs.info().file_max as usize {
            return Err(Error::FileTooLarge);
        }
        if end > self.fs.fs.inline_threshold() && self.pos == self.data.len() {
            self.start_streaming_from_buffer()?;
            self.write_streaming(data)?;
            return Ok(data.len());
        }
        if end > self.data.len() {
            self.data.resize(end, 0);
        }
        self.data[self.pos..end].copy_from_slice(data);
        self.pos = end;
        if self.data.len() > self.fs.fs.inline_threshold() && self.pos == self.data.len() {
            self.start_streaming_from_buffer()?;
        }
        Ok(data.len())
    }

    pub fn write_all(&mut self, data: &[u8]) -> Result<()> {
        self.write(data)?;
        Ok(())
    }

    pub fn seek(&mut self, pos: usize) -> Result<()> {
        if let Some(stream) = &self.stream {
            if pos != stream.len {
                return Err(Error::Unsupported);
            }
        }
        self.pos = pos;
        Ok(())
    }

    pub fn close(self) -> Result<()> {
        match self.stream {
            Some(stream) => self.fs.finish_streaming_create_file(self.path, stream),
            None => self.fs.create_file(&self.path, &self.data),
        }
    }

    fn start_streaming_from_buffer(&mut self) -> Result<()> {
        let buffered = core::mem::take(&mut self.data);
        self.stream = Some(StreamingWrite {
            allocator: self.fs.allocator.clone(),
            blocks: Vec::new(),
            current: None,
            len: 0,
            target: StreamingTarget::Create,
        });
        self.pos = 0;
        self.write_streaming(&buffered)
    }

    fn write_streaming(&mut self, mut data: &[u8]) -> Result<()> {
        let stream = self.stream.as_mut().ok_or(Error::Corrupt)?;
        if self.pos != stream.len {
            return Err(Error::Unsupported);
        }
        if stream.len.checked_add(data.len()).ok_or(Error::NoSpace)?
            > self.fs.info().file_max as usize
        {
            return Err(Error::FileTooLarge);
        }

        while !data.is_empty() {
            if stream.current.is_none() {
                let index = stream.blocks.len();
                let block = stream.allocator.alloc_block()?;
                let mut bytes = alloc::vec![0xff; self.fs.info().block_size as usize];
                let data_start = ctz_data_start(index)?;
                if index > 0 {
                    let skips = index.trailing_zeros() as usize + 1;
                    for skip in 0..skips {
                        let target_index =
                            index.checked_sub(1usize << skip).ok_or(Error::Corrupt)?;
                        let target = stream
                            .blocks
                            .get(target_index)
                            .copied()
                            .ok_or(Error::Corrupt)?;
                        program_nor_bytes(&mut bytes, skip * 4, &target.to_le_bytes())?;
                    }
                }
                stream.current = Some(StreamingBlock {
                    block,
                    bytes,
                    off: data_start,
                    mode: StreamingBlockMode::New,
                });
            }

            let current = stream.current.as_mut().ok_or(Error::Corrupt)?;
            let capacity = current.bytes.len().saturating_sub(current.off);
            let n = core::cmp::min(capacity, data.len());
            current.bytes[current.off..current.off + n].copy_from_slice(&data[..n]);
            current.off += n;
            stream.len = stream.len.checked_add(n).ok_or(Error::NoSpace)?;
            self.pos = stream.len;
            data = &data[n..];

            if current.off == current.bytes.len() {
                let current = stream.current.take().ok_or(Error::Corrupt)?;
                self.fs.flush_streaming_block(stream, current)?;
            }
        }

        Ok(())
    }
}

impl FileOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read(mut self, value: bool) -> Self {
        self.read = value;
        self
    }

    pub fn write(mut self, value: bool) -> Self {
        self.write = value;
        self
    }

    pub fn create(mut self, value: bool) -> Self {
        self.create = value;
        self
    }

    pub fn create_new(mut self, value: bool) -> Self {
        self.create_new = value;
        self
    }

    pub fn truncate(mut self, value: bool) -> Self {
        self.truncate = value;
        self
    }

    pub fn append(mut self, value: bool) -> Self {
        self.append = value;
        self
    }
}

impl<'fs, D: BlockDevice + 'static> FileHandle<'fs, D> {
    pub fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        if !self.readable {
            return Err(Error::BadFileDescriptor);
        }
        if self.stream.is_some() {
            return Err(Error::Unsupported);
        }
        if self.merge.is_some() {
            return Err(Error::Unsupported);
        }
        if self.stream_read {
            let source = self.stream_source.as_ref().ok_or(Error::Corrupt)?;
            let n = self.fs.read_file_data_at(source, self.pos, out)?;
            self.pos += n;
            return Ok(n);
        }
        let available = self.data.len().saturating_sub(self.pos);
        let n = core::cmp::min(out.len(), available);
        out[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }

    pub fn write(&mut self, data: &[u8]) -> Result<usize> {
        if !self.writable {
            return Err(Error::BadFileDescriptor);
        }
        if self.stream.is_some() {
            self.write_streaming(data)?;
            return Ok(data.len());
        }
        if self.merge.is_some() {
            return self.write_merge_patch(data);
        }
        let end = self.pos.checked_add(data.len()).ok_or(Error::NoSpace)?;
        if end > self.fs.info().file_max as usize {
            return Err(Error::FileTooLarge);
        }
        if end > self.fs.fs.inline_threshold() && self.pos == self.data.len() {
            self.start_streaming_from_buffer()?;
            self.write_streaming(data)?;
            return Ok(data.len());
        }
        if end > self.data.len() {
            self.data.resize(end, 0);
        }
        self.data[self.pos..end].copy_from_slice(data);
        self.pos = end;
        self.len = self.data.len();
        self.dirty = true;
        if self.data.len() > self.fs.fs.inline_threshold() && self.pos == self.data.len() {
            self.start_streaming_from_buffer()?;
        }
        Ok(data.len())
    }

    pub fn write_all(&mut self, data: &[u8]) -> Result<()> {
        self.write(data)?;
        Ok(())
    }

    pub fn seek(&mut self, pos: usize) -> Result<()> {
        if let Some(stream) = &self.stream {
            if pos != stream.len {
                return Err(Error::Unsupported);
            }
        }
        self.pos = pos;
        Ok(())
    }

    pub fn truncate(&mut self, len: usize) -> Result<()> {
        if !self.writable {
            return Err(Error::BadFileDescriptor);
        }
        if len > self.fs.info().file_max as usize {
            return Err(Error::Unsupported);
        }
        if self.stream.is_some() {
            return Err(Error::Unsupported);
        }
        if self.merge.is_some() {
            self.promote_merge_to_buffer()?;
        }
        self.data.resize(len, 0);
        if self.pos > len {
            self.pos = len;
        }
        self.len = len;
        self.dirty = true;
        Ok(())
    }

    pub fn flush(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        // Try writeback with a cloned plan first. A sync fault can happen after
        // data or metadata reached the device; keeping the live handle state
        // intact lets callers retry the same logical flush.
        if let Some(merge) = self.merge.clone() {
            self.flush_merge_patches(merge)?;
            self.data = self.fs.read_file(&self.path)?;
            self.len = self.data.len();
            self.pos = self.len;
            self.stream_target = StreamingTarget::Replace;
            self.merge = None;
            self.dirty = false;
            return Ok(());
        }
        if let Some(stream) = self.stream.clone() {
            let len = stream.len;
            self.fs.finish_streaming_file(self.path.clone(), stream)?;
            // Streaming handles are enabled only for write-only create,
            // truncate-replace, and append shapes. Re-reading the just-written
            // CTZ file here used to make close/sync briefly allocate the full
            // payload again, which defeated the point of streaming writes.
            self.data.clear();
            self.len = len;
            self.pos = self.len;
            self.stream_target = StreamingTarget::Replace;
            self.stream = None;
            self.dirty = false;
            return Ok(());
        }
        self.fs.write_file(&self.path, &self.data)?;
        self.len = self.data.len();
        self.stream_target = StreamingTarget::Replace;
        self.dirty = false;
        Ok(())
    }

    pub fn sync(&mut self) -> Result<()> {
        self.flush()?;
        self.fs.sync()
    }

    pub fn close(mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        if let Some(merge) = self.merge.take() {
            return self.flush_merge_patches(merge);
        }
        if let Some(stream) = self.stream.take() {
            return self.fs.finish_streaming_file(self.path, stream);
        }
        self.fs.write_file(&self.path, &self.data)
    }

    fn start_streaming_from_buffer(&mut self) -> Result<()> {
        let buffered = core::mem::take(&mut self.data);
        self.stream = Some(StreamingWrite {
            allocator: self.fs.allocator.clone(),
            blocks: Vec::new(),
            current: None,
            len: 0,
            target: self.stream_target,
        });
        self.pos = 0;
        self.write_streaming(&buffered)
    }

    fn write_merge_patch(&mut self, data: &[u8]) -> Result<usize> {
        let end = self.pos.checked_add(data.len()).ok_or(Error::NoSpace)?;
        if end > self.fs.info().file_max as usize {
            return Err(Error::FileTooLarge);
        }
        let original_len = self.merge.as_ref().ok_or(Error::Corrupt)?.original_len;
        if end > original_len {
            // Extending at an arbitrary offset needs sparse-zero semantics and
            // read-after-write merging. Keep that less common shape correct by
            // falling back to the existing buffered path.
            self.promote_merge_to_buffer()?;
            return self.write(data);
        }

        let merge = self.merge.as_mut().ok_or(Error::Corrupt)?;
        merge.patches.push(FilePatch {
            off: self.pos,
            data: data.to_vec(),
        });
        self.pos = end;
        self.len = original_len;
        self.dirty = true;
        Ok(data.len())
    }

    fn promote_merge_to_buffer(&mut self) -> Result<()> {
        let Some(merge) = self.merge.take() else {
            return Ok(());
        };
        let mut data = self.fs.read_file(&self.path)?;
        for patch in merge.patches {
            let end = patch
                .off
                .checked_add(patch.data.len())
                .ok_or(Error::NoSpace)?;
            if end > data.len() {
                data.resize(end, 0);
            }
            data[patch.off..end].copy_from_slice(&patch.data);
        }
        self.len = data.len();
        self.data = data;
        Ok(())
    }

    fn flush_merge_patches(&mut self, merge: MergeWrite) -> Result<()> {
        let mut stream = StreamingWrite {
            allocator: self.fs.allocator.clone(),
            blocks: Vec::new(),
            current: None,
            len: 0,
            target: StreamingTarget::Replace,
        };

        let mut off = 0usize;
        let mut buf = alloc::vec![0; self.fs.info().block_size as usize];
        while off < merge.original_len {
            let n = core::cmp::min(buf.len(), merge.original_len - off);
            let read = self.fs.read_file_at(&self.path, off, &mut buf[..n])?;
            if read != n {
                return Err(Error::Corrupt);
            }
            for patch in &merge.patches {
                let patch_end = patch
                    .off
                    .checked_add(patch.data.len())
                    .ok_or(Error::NoSpace)?;
                let start = core::cmp::max(off, patch.off);
                let end = core::cmp::min(off + n, patch_end);
                if start < end {
                    let dst = start - off;
                    let src = start - patch.off;
                    let len = end - start;
                    buf[dst..dst + len].copy_from_slice(&patch.data[src..src + len]);
                }
            }
            self.write_streaming_into(&mut stream, &buf[..n])?;
            off += n;
        }

        if let Some(current) = stream.current.take() {
            self.fs.flush_streaming_block(&mut stream, current)?;
        }
        self.fs.finish_streaming_file(self.path.clone(), stream)
    }

    fn write_streaming_into(&mut self, stream: &mut StreamingWrite, mut data: &[u8]) -> Result<()> {
        while !data.is_empty() {
            if stream.current.is_none() {
                let index = stream.blocks.len();
                let block = stream.allocator.alloc_block()?;
                let mut bytes = alloc::vec![0xff; self.fs.info().block_size as usize];
                let data_start = ctz_data_start(index)?;
                if index > 0 {
                    let skips = index.trailing_zeros() as usize + 1;
                    for skip in 0..skips {
                        let target_index =
                            index.checked_sub(1usize << skip).ok_or(Error::Corrupt)?;
                        let target = stream
                            .blocks
                            .get(target_index)
                            .copied()
                            .ok_or(Error::Corrupt)?;
                        program_nor_bytes(&mut bytes, skip * 4, &target.to_le_bytes())?;
                    }
                }
                stream.current = Some(StreamingBlock {
                    block,
                    bytes,
                    off: data_start,
                    mode: StreamingBlockMode::New,
                });
            }

            let current = stream.current.as_mut().ok_or(Error::Corrupt)?;
            let capacity = current.bytes.len().saturating_sub(current.off);
            let n = core::cmp::min(capacity, data.len());
            current.bytes[current.off..current.off + n].copy_from_slice(&data[..n]);
            current.off += n;
            stream.len = stream.len.checked_add(n).ok_or(Error::NoSpace)?;
            data = &data[n..];

            if current.off == current.bytes.len() {
                let current = stream.current.take().ok_or(Error::Corrupt)?;
                self.fs.flush_streaming_block(stream, current)?;
            }
        }
        Ok(())
    }

    fn write_streaming(&mut self, mut data: &[u8]) -> Result<()> {
        let stream = self.stream.as_mut().ok_or(Error::Corrupt)?;
        if self.pos != stream.len {
            return Err(Error::Unsupported);
        }
        if stream.len.checked_add(data.len()).ok_or(Error::NoSpace)?
            > self.fs.info().file_max as usize
        {
            return Err(Error::FileTooLarge);
        }

        while !data.is_empty() {
            if stream.current.is_none() {
                let index = stream.blocks.len();
                let block = stream.allocator.alloc_block()?;
                let mut bytes = alloc::vec![0xff; self.fs.info().block_size as usize];
                let data_start = ctz_data_start(index)?;
                if index > 0 {
                    let skips = index.trailing_zeros() as usize + 1;
                    for skip in 0..skips {
                        let target_index =
                            index.checked_sub(1usize << skip).ok_or(Error::Corrupt)?;
                        let target = stream
                            .blocks
                            .get(target_index)
                            .copied()
                            .ok_or(Error::Corrupt)?;
                        program_nor_bytes(&mut bytes, skip * 4, &target.to_le_bytes())?;
                    }
                }
                stream.current = Some(StreamingBlock {
                    block,
                    bytes,
                    off: data_start,
                    mode: StreamingBlockMode::New,
                });
            }

            let current = stream.current.as_mut().ok_or(Error::Corrupt)?;
            let capacity = current.bytes.len().saturating_sub(current.off);
            let n = core::cmp::min(capacity, data.len());
            current.bytes[current.off..current.off + n].copy_from_slice(&data[..n]);
            current.off += n;
            stream.len = stream.len.checked_add(n).ok_or(Error::NoSpace)?;
            self.pos = stream.len;
            self.len = stream.len;
            self.dirty = true;
            data = &data[n..];

            if current.off == current.bytes.len() {
                let current = stream.current.take().ok_or(Error::Corrupt)?;
                self.fs.flush_streaming_block(stream, current)?;
            }
        }

        Ok(())
    }
}

impl<'fs, 'a> DirHandle<'fs, 'a> {
    pub fn read(&mut self) -> Result<Option<DirEntry>> {
        let entry = self.fs.dir_entry_at(self.head, self.pos)?;
        if entry.is_some() {
            self.pos += 1;
        }
        Ok(entry)
    }

    pub fn seek(&mut self, pos: usize) -> Result<()> {
        self.pos = pos;
        Ok(())
    }

    pub fn rewind(&mut self) -> Result<()> {
        self.pos = 0;
        Ok(())
    }

    pub fn close(self) -> Result<()> {
        Ok(())
    }
}
