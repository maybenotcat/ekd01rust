//! ELF32 load-image extraction. Debug sections are deliberately not packaged.
use crate::{Result, read};
use std::path::Path;

pub const ENTRY: u32 = 0x1005_0520;
pub const STORAGE_BASE: u32 = 0x1050_0000;

fn word(bytes: &[u8], offset: usize) -> Result<u32> {
    let slice = bytes
        .get(offset..offset.checked_add(4).ok_or("ELF offset overflow")?)
        .ok_or("Truncated ELF header")?;
    Ok(u32::from_le_bytes(slice.try_into().unwrap()))
}
fn half(bytes: &[u8], offset: usize) -> Result<usize> {
    let slice = bytes
        .get(offset..offset + 2)
        .ok_or("Truncated ELF header")?;
    Ok(u16::from_le_bytes(slice.try_into().unwrap()) as usize)
}

pub fn payload(path: &Path) -> Result<Vec<u8>> {
    extract(&read(path)?)
}

fn extract(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.get(..7) != Some(b"\x7fELF\x01\x01\x01") {
        return Err("Expected ELF32 little-endian".into());
    }
    if half(bytes, 18)? != 40 || word(bytes, 24)? != ENTRY {
        return Err("Expected ARM entry at original BL handoff".into());
    }
    let phoff = word(bytes, 28)? as usize;
    let phsize = half(bytes, 42)?;
    let phcount = half(bytes, 44)?;
    if phsize < 32 {
        return Err("Invalid ELF program-header size".into());
    }
    let mut segments = Vec::new();
    for index in 0..phcount {
        let at = phoff
            .checked_add(index.checked_mul(phsize).ok_or("ELF offset overflow")?)
            .ok_or("ELF offset overflow")?;
        let header = bytes
            .get(at..at.checked_add(32).ok_or("ELF offset overflow")?)
            .ok_or("Truncated ELF program header")?;
        let size = word(header, 16)? as usize;
        if word(header, 0)? != 1 || size == 0 {
            continue;
        }
        let offset = word(header, 4)? as usize;
        let virtual_address = word(header, 8)?;
        let physical = word(header, 12)?;
        let flags = word(header, 24)?;
        let block = bytes
            .get(offset..offset.checked_add(size).ok_or("ELF offset overflow")?)
            .ok_or("Truncated ELF segment")?;
        let end = (physical as u64) + size as u64;
        if end <= ENTRY as u64 || physical >= STORAGE_BASE {
            if flags & 1 != 0 && physical >= STORAGE_BASE && virtual_address == physical {
                return Err("Executable segment outside protected XIP region".into());
            }
            continue;
        }
        if end > STORAGE_BASE as u64 {
            return Err("ELF load data overlaps littlefs partition".into());
        }
        let begin = physical.max(ENTRY);
        segments.push((begin, &block[(begin - physical) as usize..]));
    }
    if segments.iter().map(|(begin, _)| *begin).min() != Some(ENTRY) {
        return Err("Missing vector payload".into());
    }
    let end = segments
        .iter()
        .map(|(begin, block)| *begin as usize + block.len())
        .max()
        .unwrap();
    let mut payload = vec![0xff; end - ENTRY as usize];
    for (begin, block) in segments {
        let offset = (begin - ENTRY) as usize;
        payload[offset..offset + block.len()].copy_from_slice(block);
    }
    if payload.get(..4) != Some(b"\x18\xf0\x9f\xe5") {
        return Err("Unexpected ARM absolute vector instruction".into());
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 88];
        bytes[..7].copy_from_slice(b"\x7fELF\x01\x01\x01");
        bytes[18..20].copy_from_slice(&40u16.to_le_bytes());
        bytes[24..28].copy_from_slice(&ENTRY.to_le_bytes());
        bytes[28..32].copy_from_slice(&52u32.to_le_bytes());
        bytes[42..44].copy_from_slice(&32u16.to_le_bytes());
        bytes[44..46].copy_from_slice(&1u16.to_le_bytes());
        for (offset, value) in [
            (52, 1),
            (56, 84),
            (60, ENTRY),
            (64, ENTRY),
            (68, 4),
            (76, 5),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[84..88].copy_from_slice(b"\x18\xf0\x9f\xe5");
        bytes
    }
    #[test]
    fn load_segments_exclude_debug_and_reject_out_of_bounds_data() {
        let mut bytes = fixture();
        bytes.extend_from_slice(b"unloaded debug data");
        assert_eq!(extract(&bytes).unwrap(), b"\x18\xf0\x9f\xe5");
        bytes[68..72].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(
            extract(&bytes)
                .unwrap_err()
                .contains("Truncated ELF segment")
        );
        assert!(extract(&bytes[..30]).is_err());
    }
    #[test]
    fn storage_boundary_and_vector_instruction_are_enforced() {
        let mut bytes = fixture();
        bytes[64..68].copy_from_slice(&(STORAGE_BASE - 2).to_le_bytes());
        assert!(extract(&bytes).unwrap_err().contains("overlaps littlefs"));
        let mut bytes = fixture();
        bytes[64..68].copy_from_slice(&STORAGE_BASE.to_le_bytes());
        bytes[60..64].copy_from_slice(&STORAGE_BASE.to_le_bytes());
        assert!(
            extract(&bytes)
                .unwrap_err()
                .contains("outside protected XIP")
        );
        let mut bytes = fixture();
        bytes[84] = 0;
        assert!(extract(&bytes).unwrap_err().contains("vector instruction"));
    }
}
