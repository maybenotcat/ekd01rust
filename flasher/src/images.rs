//! Strict EKD01 scatter/GFH planning, performed before opening any port.
use crate::Result;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

pub const FLASH_BYTES: u32 = 0x800000;
pub const SETTINGS_BEGIN: u32 = 0x500000;
pub struct Image {
    pub name: String,
    pub bytes: Vec<u8>,
    pub kind: u16,
    pub load: u32,
    pub physical: u32,
    pub content: u32,
    pub jump: u32,
    pub attributes: u32,
}
fn le32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
impl Image {
    pub fn parse(name: String, bytes: Vec<u8>) -> Result<Self> {
        if bytes.len() < 56 || &bytes[..4] != b"MMM\x01" || &bytes[8..17] != b"FILE_INFO" {
            return Err(format!("{name}: expected GFH FILE_INFO"));
        }
        if le32(&bytes, 32) as usize != bytes.len() || bytes[26] != 7 {
            return Err(format!("{name}: size mismatch or non-SF image"));
        }
        let kind = u16::from_le_bytes(bytes[24..26].try_into().unwrap());
        // Original BL/EXT_BL retain their 32-byte HASH tail (sig_type=1).
        // They are accepted unchanged and separately checked against N22 hashes.
        if bytes[27] != if kind == 0x100 { 0 } else { 1 } {
            return Err(format!("{name}: unexpected GFH integrity type"));
        }
        let load = le32(&bytes, 28);
        let physical = match (kind, load) {
            (1, 0x70006000) => 0x800,
            (2, 0x10027000) => 0x27000,
            (0x100, 0x10050000) => 0x50000,
            _ => {
                return Err(format!(
                    "{name}: unsupported EKD01 GFH type/address {kind:04X}/{load:08X}"
                ));
            }
        };
        if physical as usize + bytes.len() > SETTINGS_BEGIN as usize || bytes.len() % 2 != 0 {
            return Err(format!(
                "{name}: image is unaligned or overlaps the private settings partition"
            ));
        }
        let content = le32(&bytes, 40);
        let jump = le32(&bytes, 48);
        let attributes = le32(&bytes, 52);
        if content as usize >= bytes.len() || jump as usize >= bytes.len() {
            return Err(format!("{name}: GFH entry/content outside file"));
        }
        if kind == 0x100
            && (content != 0x520
                || jump != 0x520
                || bytes.get(0x520..0x524) != Some(b"\x18\xf0\x9f\xe5"))
        {
            return Err(format!("{name}: unexpected application entry layout"));
        }
        Ok(Self {
            name,
            bytes,
            kind,
            load,
            physical,
            content,
            jump,
            attributes,
        })
    }
    // DA 0x51 descriptor, confirmed in original sub_1002918E. Numbers on wire
    // are big-endian, despite the little-endian GFH file.
    pub fn boot_descriptor(&self) -> Vec<u8> {
        let mut out = vec![0x5a, 7]; // enabled, GFH_FLASH_DEV_SF
        out.extend(self.kind.to_be_bytes());
        for value in [
            self.load,
            self.load + self.bytes.len() as u32,
            self.jump,
            self.content,
        ] {
            out.extend(value.to_be_bytes());
        }
        out.push(0); // no encrypted range
        for value in [0, 0, self.bytes.len() as u32] {
            out.extend(value.to_be_bytes());
        }
        out.extend([0, 2]); // no auxiliary ROM flags, block alignment
        out.extend(self.attributes.to_be_bytes());
        out
    }
}
pub struct Kit {
    pub images: Vec<Image>,
    pub scatter: PathBuf,
}
impl Kit {
    pub fn load(input: &Path) -> Result<Self> {
        let scatter = if input.is_dir() {
            input.join("SOUTHBASE2502C_WT_11C_BB.cfg")
        } else {
            input.to_path_buf()
        };
        let directory = scatter.parent().ok_or("Scatter has no parent directory")?;
        let text =
            fs::read_to_string(&scatter).map_err(|e| format!("{}: {e}", scatter.display()))?;
        if !text
            .lines()
            .any(|line| line.split('#').next().unwrap().trim() == "platform: MT2502")
        {
            return Err("Only MT2502 EKD01 scatter is supported".into());
        }
        let mut enabled = false;
        let mut images = Vec::new();
        for line in text.lines() {
            let line = line.split('#').next().unwrap();
            if !line.starts_with(char::is_whitespace) && line.trim_end().ends_with(':') {
                enabled = matches!(line.trim(), "boot_region:" | "main_region:");
            }
            if enabled && let Some(name) = line.trim().strip_prefix("- file:") {
                let name = name.trim().trim_matches(['\'', '"']);
                if name.is_empty()
                    || Path::new(name)
                        .components()
                        .any(|p| !matches!(p, Component::Normal(_)))
                {
                    return Err("Invalid scatter filename".into());
                }
                let bytes = fs::read(directory.join(name)).map_err(|e| format!("{name}: {e}"))?;
                images.push(Image::parse(name.into(), bytes)?);
            }
        }
        images.sort_by_key(|image| image.kind);
        if images.iter().map(|image| image.kind).collect::<Vec<_>>() != [1, 2, 0x100] {
            return Err("Scatter must contain exactly ARM_BL, ARM_EXT_BL and PRIMARY_MAUI".into());
        }
        // The physical BL location differs from its RAM load address. Only
        // unchanged original N22 loaders are accepted for a full download.
        for (image, expected) in images.iter().take(2).zip([
            "b10cc9bfc364662b97937999cfc444bc689c9612c824ac89c98b69be0a58dd77",
            "e7d49e028cd47eb81e4e29a61ff4b09dcafb5288577aa501556fa5754139c481",
        ]) {
            if format!("{:x}", Sha256::digest(&image.bytes)) != expected {
                return Err(format!("{}: original loader checksum mismatch", image.name));
            }
        }
        if let Ok(info) = fs::read(directory.join("image.json")) {
            let info: serde_json::Value =
                serde_json::from_slice(&info).map_err(|e| e.to_string())?;
            let main = &images[2];
            if info["sha256"].as_str() != Some(&format!("{:x}", Sha256::digest(&main.bytes)))
                || info["bytes"].as_u64() != Some(main.bytes.len() as u64)
            {
                return Err("Application does not match image.json".into());
            }
        }
        for pair in images.windows(2) {
            if pair[0].physical as usize + pair[0].bytes.len() > pair[1].physical as usize {
                return Err("Images overlap".into());
            }
        }
        Ok(Self { images, scatter })
    }
    pub fn print(&self, bootloaders: bool) {
        println!("Scatter: {}", self.scatter.display());
        for image in &self.images {
            println!(
                "{} {:08X}..{:08X} ({:7} bytes) {}",
                if bootloaders || image.kind == 0x100 {
                    "WRITE"
                } else {
                    "KEEP "
                },
                image.physical,
                image.physical as usize + image.bytes.len(),
                image.bytes.len(),
                image.name
            );
        }
        println!("KEEP  00500000..00800000 settings/filesystem; auxiliary MCU is not accessed");
    }
}

#[test]
fn checked_in_loaders_have_their_real_flash_locations() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let main = fs::read(root.join("firmware/original/EXT_BOOTLOADER")).unwrap();
    let image = Image::parse("EXT_BOOTLOADER".into(), main).unwrap();
    assert_eq!(image.physical, 0x27000);
    assert_eq!(image.boot_descriptor().len(), 39);
    assert_eq!(image.content, 0x60);
}
