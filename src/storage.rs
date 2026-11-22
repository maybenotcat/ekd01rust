use crate::{
    crc::crc32,
    soc::{
        DebugUart,
        flash::{CLAIM, Flash, FlashError, SECTOR_BYTES, STORAGE, STORAGE_END},
    },
};
use alloc::{rc::Rc, vec::Vec};
use core::cell::RefCell;
use littlefs_rust::{
    BlockDevice, Config, Error, Filesystem, FilesystemMut, FilesystemOptions, InlineMax,
};

pub type FlashBus = Rc<RefCell<Flash>>;
const BLOCKS: usize = (STORAGE_END - STORAGE) as usize / SECTOR_BYTES;
const CLAIM_MAGIC: u32 = 0x53464b45;

/// Share the serial NOR controller without duplicating ownership or peripheral setup.
#[derive(Clone)]
pub struct NorBlockDevice {
    bus: FlashBus,
}
impl NorBlockDevice {
    fn io_error(operation: &str, address: u32, error: FlashError) -> Error {
        DebugUart::log(operation);
        DebugUart::hex("[FLASH] ERROR_ADDRESS", address);
        // FlashError discriminants follow the declaration in soc/flash.rs.
        DebugUart::hex("[FLASH] ERROR_KIND", error as u32);
        Error::Io
    }
    pub fn new(bus: FlashBus) -> Self {
        Self { bus }
    }
    fn offset(block: u32, off: usize, len: usize) -> Result<u32, Error> {
        if block as usize >= BLOCKS || off > SECTOR_BYTES || len > SECTOR_BYTES - off {
            return Err(Error::OutOfBounds);
        }
        Ok(STORAGE + block * SECTOR_BYTES as u32 + off as u32)
    }
}
impl BlockDevice for NorBlockDevice {
    fn config(&self) -> Config {
        Config {
            block_size: SECTOR_BYTES,
            block_count: BLOCKS,
        }
    }
    fn read(&self, block: u32, off: usize, out: &mut [u8]) -> Result<(), Error> {
        let offset = Self::offset(block, off, out.len())?;
        self.bus
            .borrow()
            .read(offset, out)
            .map_err(|error| Self::io_error("[FLASH] READ_FAILED\n", offset, error))
    }
    fn prog(&mut self, block: u32, off: usize, data: &[u8]) -> Result<(), Error> {
        let offset = Self::offset(block, off, data.len())?;
        self.bus
            .borrow_mut()
            .program(offset, data)
            .map_err(|error| Self::io_error("[FLASH] PROGRAM_FAILED\n", offset, error))
    }
    fn erase(&mut self, block: u32) -> Result<(), Error> {
        let offset = Self::offset(block, 0, SECTOR_BYTES)?;
        self.bus
            .borrow_mut()
            .erase(offset)
            .map_err(|error| Self::io_error("[FLASH] ERASE_FAILED\n", offset, error))
    }
}

pub struct SettingsStore {
    bus: FlashBus,
    fs: Option<FilesystemMut<NorBlockDevice>>,
}
impl SettingsStore {
    pub fn new(bus: FlashBus) -> Self {
        Self { bus, fs: None }
    }
    fn options() -> FilesystemOptions {
        FilesystemOptions {
            read_size: 16,
            prog_size: 256,
            cache_size: 256,
            lookahead_size: 64,
            block_cycles: Some(100),
            name_max: 31,
            file_max: STORAGE_END - STORAGE,
            inline_max: InlineMax::Limit(128),
            ..FilesystemOptions::default()
        }
    }
    fn claim(&self) -> Result<[u8; 32], Error> {
        let mut bytes = [0; 32];
        self.bus
            .borrow()
            .read(CLAIM, &mut bytes)
            .map_err(|_| Error::Io)?;
        Ok(bytes)
    }
    fn owned(bytes: &[u8; 32]) -> bool {
        let word = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        word(0) == CLAIM_MAGIC
            && word(4) == 1
            && word(8) == STORAGE
            && word(12) == BLOCKS as u32
            && word(24) == crc32(&bytes[..24])
    }
    fn mount(&mut self, create: bool) -> Result<(), Error> {
        if self.fs.is_some() {
            return Ok(());
        }
        let mut claim = self.claim()?;
        if !Self::owned(&claim) {
            if !create {
                return Err(Error::NotFound);
            }
            DebugUart::log("[LFS] PRIVATE_PARTITION_INITIALIZE\n");
            claim.fill(0);
            for (offset, value) in [(0, CLAIM_MAGIC), (4, 1), (8, STORAGE), (12, BLOCKS as u32)] {
                claim[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            }
            let crc = crc32(&claim[..24]);
            claim[24..28].copy_from_slice(&crc.to_le_bytes());
            claim[28..32].fill(255);
            let mut flash = self.bus.borrow_mut();
            flash.erase(CLAIM).map_err(|_| Error::Io)?;
            flash.program(CLAIM, &claim).map_err(|_| Error::Io)?;
        }
        let state = u32::from_le_bytes(claim[28..32].try_into().unwrap());
        if state != u32::MAX && state != u32::MAX - 1 {
            return Err(Error::Corrupt);
        }
        if state == u32::MAX && !create {
            return Err(Error::NotFound);
        }
        let mut result = Filesystem::mount_device_mut_with_options(
            NorBlockDevice::new(self.bus.clone()),
            Self::options(),
        );
        if result.is_err() && state == u32::MAX && create {
            DebugUart::log("[LFS] INITIAL_FORMAT\n");
            Filesystem::format_device_lazy_with_options(
                &mut NorBlockDevice::new(self.bus.clone()),
                Self::options(),
            )?;
            result = Filesystem::mount_device_mut_with_options(
                NorBlockDevice::new(self.bus.clone()),
                Self::options(),
            );
        }
        let fs = result?;
        if state == u32::MAX {
            self.bus
                .borrow_mut()
                .program(CLAIM + 28, &(u32::MAX - 1).to_le_bytes())
                .map_err(|_| Error::Io)?;
        }
        self.fs = Some(fs);
        DebugUart::log("[LFS] MOUNTED\n");
        Ok(())
    }
    pub fn read(&mut self, path: &str) -> Result<Vec<u8>, Error> {
        if let Some(fs) = &self.fs {
            return fs.read_file(path);
        }
        let claim = self.claim()?;
        if !Self::owned(&claim) {
            return Err(Error::NotFound);
        }
        let state = u32::from_le_bytes(claim[28..32].try_into().unwrap());
        if state == u32::MAX {
            return Err(Error::NotFound);
        }
        if state != u32::MAX - 1 {
            return Err(Error::Corrupt);
        }
        // A mutable mount may repair littlefs global state and write to NOR.
        // Startup must remain read-only; defer repair to the first explicit save.
        let device = NorBlockDevice::new(self.bus.clone());
        Filesystem::mount_device_with_options(&device, Self::options())?.read_file(path)
    }
    pub fn write_verified(&mut self, path: &str, data: &[u8]) -> Result<(), Error> {
        self.mount(true)?;
        let result = (|| {
            let fs = self.fs.as_mut().unwrap();
            if path.starts_with("/prefs/") {
                match fs.create_dir("/prefs") {
                    Ok(()) | Err(Error::AlreadyExists) => {}
                    Err(error) => return Err(error),
                }
            }
            fs.write_file(path, data)?;
            if fs.read_file(path)?.as_slice() != data {
                return Err(Error::Corrupt);
            }
            Ok(())
        })();
        if result.is_err() {
            self.fs = None;
        }
        result
    }
    pub fn read_legacy(&self, offset: u32, out: &mut [u8]) -> Result<(), Error> {
        if !matches!(offset, 0x7fe000 | 0x7ff000) || out.len() > SECTOR_BYTES {
            return Err(Error::OutOfBounds);
        }
        self.bus.borrow().read(offset, out).map_err(|_| Error::Io)
    }
    pub fn detach(&mut self) {
        self.fs = None;
    }
}
