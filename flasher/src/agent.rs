//! Unmodified regions 2..5 of entry 45 in the original Varstrom DA container.
use crate::Result;
use sha2::{Digest, Sha256};

pub struct Region {
    pub bytes: &'static [u8],
    pub address: u32,
    pub signature_bytes: u32,
}
pub const STAGES: [Region; 2] = [
    Region {
        bytes: include_bytes!("../../firmware/download-agent/stage1.bin"),
        address: 0x70007000,
        signature_bytes: 0x100,
    },
    Region {
        bytes: include_bytes!("../../firmware/download-agent/stage2.bin"),
        address: 0x10020000,
        signature_bytes: 0x100,
    },
];
pub const NOR_TABLE: &[u8] = include_bytes!("../../firmware/download-agent/nor-table.bin");
pub const NAND_TABLE: &[u8] = include_bytes!("../../firmware/download-agent/nand-table.bin");
pub const MANIFEST: &[u8] = include_bytes!("../../firmware/download-agent/manifest.json");

pub fn validate() -> Result<()> {
    let manifest: serde_json::Value =
        serde_json::from_slice(MANIFEST).map_err(|e| e.to_string())?;
    let files = manifest["files"].as_array().ok_or("Invalid DA manifest")?;
    for (file, data) in files
        .iter()
        .zip([STAGES[0].bytes, STAGES[1].bytes, NOR_TABLE, NAND_TABLE])
    {
        let digest = format!("{:x}", Sha256::digest(data));
        if file["sha256"].as_str() != Some(&digest)
            || file["bytes"].as_u64() != Some(data.len() as u64)
        {
            return Err("Original DA integrity check failed".into());
        }
    }
    if files.len() != 4 || NOR_TABLE.len() % 36 != 0 {
        return Err("Invalid DA regions".into());
    }
    Ok(())
}

#[test]
fn bundled_agent_matches_original_container_manifest() {
    validate().unwrap();
}
