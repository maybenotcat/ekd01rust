//! Strict EKD01 scatter/GFH planning, performed before opening any port.
use crate::Result;
use ekd01_boot_contract as layout;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

pub const FLASH_BYTES: u32 = 0x800000;
pub const SETTINGS_BEGIN: u32 = 0x500000;
#[derive(Clone)]
pub struct Image {
    pub name: String,
    pub bytes: Vec<u8>,
    pub kind: u16,
    pub load: u32,
    pub physical: u32,
    pub content: u32,
    pub jump: u32,
    pub boot_attributes: u32,
    pub attributes: u32,
}
fn le32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

/// Check metadata produced by the original DA, independently of GFH readback.
/// DA 10029690 copies descriptor+16 into BRLYT.m_bl_attribute.
pub fn validate_boot_metadata(bytes: &[u8], images: &[Image]) -> Result<()> {
    const BRLYT: usize = 512;
    const DESCRIPTORS: usize = BRLYT + 20;
    if bytes.len() < DESCRIPTORS + 8 * 20 || images.is_empty() || images.len() > 8 {
        return Err("Incomplete boot metadata".into());
    }
    if &bytes[..8] != b"SF_BOOT\0"
        || le32(bytes, 12) != 1
        || le32(bytes, 16) != 512
        || &bytes[BRLYT..BRLYT + 8] != b"BRLYT\0\0\0"
        || le32(bytes, BRLYT + 8) != 1
    {
        return Err("Invalid SF_BOOT/BRLYT header".into());
    }
    for (index, image) in images.iter().enumerate() {
        let at = DESCRIPTORS + index * 20;
        let kind = u16::from_le_bytes(bytes[at + 6..at + 8].try_into().unwrap());
        if le32(bytes, at) != 0x42424242
            || bytes[at + 4] != 7
            || kind != image.kind
            || le32(bytes, at + 8) != image.physical
            || le32(bytes, at + 12) != image.physical + image.bytes.len() as u32
            || le32(bytes, at + 16) != image.boot_attributes
        {
            return Err(format!(
                "{}: BRLYT mismatch; attributes={:08X}, expected={:08X}",
                image.name,
                le32(bytes, at + 16),
                image.boot_attributes
            ));
        }
    }
    for index in images.len()..8 {
        if le32(bytes, DESCRIPTORS + index * 20) == 0x42424242 {
            return Err("Unexpected extra bootloader in BRLYT".into());
        }
    }
    Ok(())
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
        let mut boot_attributes = None;
        if kind != 0x100 {
            let mut at = 0usize;
            while at + 8 <= content as usize {
                if bytes.get(at..at + 3) != Some(b"MMM") {
                    return Err(format!("{name}: invalid GFH record"));
                }
                let size = u16::from_le_bytes(bytes[at + 4..at + 6].try_into().unwrap()) as usize;
                if size < 8 || at + size > content as usize {
                    return Err(format!("{name}: invalid GFH record size"));
                }
                let record_type = u16::from_le_bytes(bytes[at + 6..at + 8].try_into().unwrap());
                if record_type == 1 {
                    if size < 12 || boot_attributes.is_some() {
                        return Err(format!("{name}: invalid BL_INFO"));
                    }
                    boot_attributes = Some(le32(&bytes, at + 8));
                }
                at += size;
            }
            if boot_attributes.is_none() {
                return Err(format!("{name}: missing GFH_BL_INFO"));
            }
            if kind == 1 && boot_attributes.unwrap() & 1 == 0 {
                return Err(format!(
                    "{name}: GFH_BL_INFO does not enable LOAD_BY_BOOTROM"
                ));
            }
        }
        Ok(Self {
            name,
            bytes,
            kind,
            load,
            physical,
            content,
            jump,
            boot_attributes: boot_attributes.unwrap_or(0),
            attributes,
        })
    }
    // DA 0x51 descriptor, confirmed in sub_1002918E and consumer 10029690.
    // The third word is GFH_BL_INFO.m_bl_attr, NOT the GFH jump offset.
    // The final word is FILE_INFO attributes, a separate field. Numbers on wire
    // are big-endian, despite the little-endian GFH file.
    pub fn boot_descriptor(&self) -> Vec<u8> {
        let mut out = vec![0x5a, 7]; // enabled, GFH_FLASH_DEV_SF
        out.extend(self.kind.to_be_bytes());
        for value in [
            self.load,
            self.load + self.bytes.len() as u32,
            self.boot_attributes,
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
    pub embassy: bool,
}
impl Kit {
    pub fn staged_application(&self) -> Result<Image> {
        if !self.embassy {
            return Err("Staging requires an Embassy kit".into());
        }
        let mut image = self.images.last().unwrap().clone();
        image.physical = layout::DFU.start;
        image.name = format!("DFU/{}", image.name);
        Ok(image)
    }

    pub fn pending_marker(&self) -> Result<Image> {
        if !self.embassy {
            return Err("Staging requires an Embassy kit".into());
        }
        // DA_MEM writes a bounded byte range; these four bytes are Embassy's
        // SWAP word, not a second executable or an auxiliary MCU image.
        Ok(Image {
            name: "Embassy pending marker".into(),
            bytes: layout::PENDING_MARKER.to_vec(),
            kind: 0x100,
            load: 0,
            physical: layout::STATE.start,
            content: 0,
            jump: 0,
            boot_attributes: 0,
            attributes: 0,
        })
    }

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
        let kinds = images.iter().map(|image| image.kind).collect::<Vec<_>>();
        let embassy = kinds == [1, 0x100];
        if !embassy && kinds != [1, 2, 0x100] {
            return Err("Expected one Embassy loader and application, or original BL/EXT_BL and application".into());
        }
        // The physical BL location differs from its RAM load address. Only
        // unchanged original N22 loaders are accepted for a full download.
        for (image, expected) in images.iter().take(if embassy { 0 } else { 2 }).zip([
            "b10cc9bfc364662b97937999cfc444bc689c9612c824ac89c98b69be0a58dd77",
            "e7d49e028cd47eb81e4e29a61ff4b09dcafb5288577aa501556fa5754139c481",
        ]) {
            if format!("{:x}", Sha256::digest(&image.bytes)) != expected {
                return Err(format!("{}: original loader checksum mismatch", image.name));
            }
        }
        let info = fs::read(directory.join("image.json"));
        if embassy && info.is_err() {
            return Err("Embassy kit requires image.json".into());
        }
        if let Ok(info) = info {
            let info: serde_json::Value =
                serde_json::from_slice(&info).map_err(|e| e.to_string())?;
            let main = images.last().unwrap();
            if info["sha256"].as_str() != Some(&format!("{:x}", Sha256::digest(&main.bytes)))
                || info["bytes"].as_u64() != Some(main.bytes.len() as u64)
            {
                return Err("Application does not match image.json".into());
            }
            if embassy {
                let loader = &images[0];
                let metadata = &info["bootloader"];
                if info["boot_scheme"] != "ekd01-embassy-v1"
                    || metadata["image"].as_str() != Some(loader.name.as_str())
                    || metadata["bytes"].as_u64() != Some(loader.bytes.len() as u64)
                    || metadata["sha256"].as_str()
                        != Some(format!("{:x}", Sha256::digest(&loader.bytes)).as_str())
                    || info["boot_layout"]
                        != serde_json::json!({
                        "active": [layout::ACTIVE.start, layout::ACTIVE.end],
                        "dfu": [layout::DFU.start, layout::DFU.end],
                        "state": [layout::STATE.start, layout::STATE.end]})
                {
                    return Err("Custom bootloader/layout does not match image.json".into());
                }
                if main.bytes.len() > layout::ACTIVE.len() as usize
                    || loader.content != 0x140
                    || loader.jump != 0x140
                    || loader.attributes != 1
                    || loader.load + loader.bytes.len() as u32 > layout::BOOT_ROM_END
                {
                    return Err("Custom image exceeds boot memory layout".into());
                }
                let body = loader
                    .bytes
                    .len()
                    .checked_sub(32)
                    .ok_or("Missing BootROM checksum")?;
                if loader.bytes[27] != 1
                    || le32(&loader.bytes, 44) != 32
                    || loader.bytes[body..body + 20]
                        != sha1::Sha1::digest(&loader.bytes[..body])[..]
                    || loader.bytes[body + 20..] != [0; 12]
                {
                    return Err("Invalid BootROM SHA-1 trailer".into());
                }
                layout::validate_application(&main.bytes)
                    .map_err(|e| format!("Invalid application: {e:?}"))?;
            }
        }
        for pair in images.windows(2) {
            if pair[0].physical as usize + pair[0].bytes.len() > pair[1].physical as usize {
                return Err("Images overlap".into());
            }
        }
        Ok(Self {
            images,
            scatter,
            embassy,
        })
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
        if self.embassy {
            println!(
                "RESET {:08X}..{:08X} Embassy rollback state after verified direct download",
                layout::STATE.start,
                layout::STATE.end
            );
        }
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
    assert_eq!(image.boot_attributes, 0);
    assert_eq!(&image.boot_descriptor()[12..16], &0u32.to_be_bytes());
    assert_eq!(&image.boot_descriptor()[35..39], &3u32.to_be_bytes());
}

#[test]
fn rom_load_attribute_is_not_the_entry_offset() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let image = Image::parse(
        "BL".into(),
        fs::read(
            root.join("firmware/original/EKD01_CAN_BF_BOOTLOADER_V005_MT2502_EKD01_CAN_BF_N22.bin"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(image.boot_attributes, 1);
    assert_eq!(image.jump, 0x140);
    assert_eq!(&image.boot_descriptor()[12..16], &1u32.to_be_bytes());
    let mut metadata = vec![0; 1024];
    metadata[..8].copy_from_slice(b"SF_BOOT\0");
    metadata[512..520].copy_from_slice(b"BRLYT\0\0\0");
    for (at, value) in [
        (12, 1),
        (16, 512),
        (520, 1),
        (532, 0x42424242),
        (540, image.physical),
        (544, image.physical + image.bytes.len() as u32),
        (548, 1),
    ] {
        metadata[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    metadata[536] = 7;
    metadata[538..540].copy_from_slice(&1u16.to_le_bytes());
    let images = [image];
    assert!(validate_boot_metadata(&metadata, &images).is_ok());
    // The exact failure captured in the device readback: a valid BL hash,
    // but BRLYT attributes equal the entry offset and LOAD_BY_BOOTROM is clear.
    metadata[548..552].copy_from_slice(&0x140u32.to_le_bytes());
    assert!(
        validate_boot_metadata(&metadata, &images)
            .unwrap_err()
            .contains("attributes=00000140")
    );
}
