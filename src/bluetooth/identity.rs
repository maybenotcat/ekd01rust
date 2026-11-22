use crate::{
    crc::crc32,
    soc::flash::{Flash, FlashError},
};
const SECTOR: u32 = 0x7fd000;
const MAGIC: u32 = 0x49425445;

pub struct Identity;
impl Identity {
    fn valid(record: &[u8; 32]) -> bool {
        u32::from_le_bytes(record[..4].try_into().unwrap()) == MAGIC
            && u32::from_le_bytes(record[4..8].try_into().unwrap()) == 1
            && record[13] & 3 == 2
            && u32::from_le_bytes(record[28..].try_into().unwrap()) == crc32(&record[..28])
    }
    pub fn load_or_create(flash: &mut Flash, entropy: [u8; 6]) -> Result<[u8; 6], FlashError> {
        let mut free = None;
        for offset in (0..4096).step_by(32) {
            let mut record = [0u8; 32];
            flash.read(SECTOR + offset, &mut record)?;
            if Self::valid(&record) {
                return Ok(record[8..14].try_into().unwrap());
            }
            if record.iter().all(|byte| *byte == 255) {
                if free.is_none() {
                    free = Some(SECTOR + offset);
                }
            } else if u32::from_le_bytes(record[..4].try_into().unwrap()) != MAGIC {
                return Err(FlashError::Protected);
            }
        }
        let offset = free.ok_or(FlashError::Protected)?;
        let mut record = [0u8; 32];
        record[..4].copy_from_slice(&MAGIC.to_le_bytes());
        record[4..8].copy_from_slice(&1u32.to_le_bytes());
        record[8..14].copy_from_slice(&entropy);
        record[13] = (record[13] & 0xfc) | 2;
        let crc = crc32(&record[..28]);
        record[28..].copy_from_slice(&crc.to_le_bytes());
        flash.program(offset, &record)?;
        flash.read(offset, &mut record)?;
        if !Self::valid(&record) {
            return Err(FlashError::Verify);
        }
        Ok(record[8..14].try_into().unwrap())
    }
}
