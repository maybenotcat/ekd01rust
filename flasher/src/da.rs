//! Wire protocol confirmed against the original DA (addresses in docs).
use crate::{
    Result, agent,
    images::Image,
    wire::{ACK, CONTINUE, NACK, Wire, sum16},
};
use std::{
    io::{Read, Write},
    time::Duration,
};

pub const BLOCK: usize = 4096;
pub struct DownloadAgent<T> {
    pub wire: Wire<T>,
    pub flash_bytes: u32,
}
impl<T: Read + Write> DownloadAgent<T> {
    pub fn initialize(mut wire: Wire<T>) -> Result<Self> {
        wire.phase = "DA initialization";
        wire.timeout = Duration::from_secs(60);
        let version = wire.receive(4)?;
        if version[..3] != [0xc0, 3, 2] || version[3] == 0xff {
            return Err(format!("Unexpected original DA version: {version:02X?}"));
        }
        println!(
            "DA: {}.{}; baseband={:02X}",
            version[1], version[2], version[3]
        );
        // Download-without-battery enabled, ROM/BL versions, CS0 with decoder,
        // NAND ACCCON, 26 MHz clock and original host setup suffix.
        wire.send(&[ACK, 5, 0xfe, 0, 8, 0, 0x70, 7, 0xff, 0xff, 2, 0, 0, 1, 8])?;
        let setup = wire.byte()?;
        if ![ACK, NACK].contains(&setup) {
            return Err(format!("DA setup response: {setup:02X}"));
        }
        let mut accepted = false;
        for record in agent::NOR_TABLE.chunks_exact(36) {
            if record[..4] == [0xff, 0xff, 0, 0] {
                break;
            }
            wire.send(record)?;
            match wire.byte()? {
                ACK => {
                    accepted = true;
                    break;
                }
                CONTINUE => {}
                response => return Err(format!("DA NOR table response: {response:02X}")),
            }
        }
        if !accepted {
            return Err("Flash chip is absent from the original DA table".into());
        }
        wire.expect(&[NACK, CONTINUE])?;
        wire.put32(0)?; // BMT pool, unused for NOR
        let marker = wire.be32()?;
        if marker != 0x0ca2 {
            return Err(format!("Original DA initialization marker: {marker:08X}"));
        }
        // sub_10025C12 sends exactly 188 bytes, including its final ACK.
        let info = wire.receive(188)?;
        if info[187] != ACK {
            return Err("Malformed original DA flash-info trailer".into());
        }
        let status = u32::from_be_bytes(info[..4].try_into().unwrap());
        let flash_bytes = u32::from_be_bytes(info[8..12].try_into().unwrap());
        let id: Vec<u16> = info[16..24]
            .chunks_exact(2)
            .map(|v| u16::from_be_bytes(v.try_into().unwrap()))
            .collect();
        println!("NOR: status={status:08X}, size={flash_bytes:08X}, IDs={id:04X?}");
        if status != 0 || flash_bytes != crate::images::FLASH_BYTES {
            return Err("Expected the EKD01 8 MiB NOR flash".into());
        }
        wire.ack()?; // sub_1002EC2A: pass-info marker
        let download_status = wire.be32()?;
        let boot_style = wire.be32()?;
        if download_status != 0 || boot_style != 0x407 {
            return Err(format!(
                "DA status/style {download_status:08X}/{boot_style:08X}; expected S_SF_FLASH_BOOT=0x407"
            ));
        }
        // Original SRAM stage sub_70007128 emits C1 when ready; no port reset.
        wire.expect(&[0xc1])?;
        wire.send(&[0x59])?;
        wire.expect(&[NACK])?;
        println!("DA READY; original loader; no flash operation performed yet");
        Ok(Self { wire, flash_bytes })
    }
    #[cfg(test)]
    pub fn from_ready(wire: Wire<T>, flash_bytes: u32) -> Self {
        Self { wire, flash_bytes }
    }
    fn range(&self, address: u32, length: u32) -> Result<()> {
        if length == 0
            || address
                .checked_add(length)
                .is_none_or(|end| end > self.flash_bytes)
        {
            return Err("NOR range outside flash".into());
        }
        Ok(())
    }
    pub fn read(
        &mut self,
        address: u32,
        length: u32,
        mut consume: impl FnMut(&[u8]) -> Result<()>,
    ) -> Result<()> {
        self.range(address, length)?;
        self.wire.phase = "DA NOR read/verify";
        self.wire.timeout = Duration::from_secs(15);
        // The original USB DA drops a non-USB-packet-aligned tail in
        // sub_100256C6, while still sending padding for the requested size.
        // Request whole BLOCKs instead and trim the data only on the host.
        // Align both ends so even an unaligned read at the end of flash stays
        // inside the physical device and every DA packet is completely full.
        let block = BLOCK as u32;
        let wire_start = address / block * block;
        let requested_end = address + length;
        let wire_end = requested_end.div_ceil(block) * block;
        self.wire.send(&[0xd6, 0])?; // NOR storage, no host-OS byte on this DA
        self.wire.put32(wire_start)?;
        self.wire.put32(wire_end - wire_start)?;
        self.wire.ack()?;
        // sub_100263DA reads packet size after acknowledging the range.
        self.wire.put32(BLOCK as u32)?;
        let mut cursor = wire_start;
        while cursor < wire_end {
            let data = self.wire.receive(BLOCK)?;
            let checksum = self.wire.be16()?;
            if checksum != sum16(&data) {
                self.wire.send(&[NACK])?;
                return Err("NOR read checksum mismatch".into());
            }
            let begin = address.saturating_sub(cursor).min(block) as usize;
            let end = requested_end.saturating_sub(cursor).min(block) as usize;
            if begin < end {
                consume(&data[begin..end])?;
            }
            self.wire.send(&[ACK])?;
            cursor += block;
        }
        Ok(())
    }
    pub fn verify(&mut self, image: &Image) -> Result<()> {
        let mut offset = 0;
        let mut progress = 0;
        println!("VERIFY {}: 0%", image.name);
        self.read(image.physical, image.bytes.len() as u32, |bytes| {
            if image.bytes[offset..offset + bytes.len()] != *bytes {
                return Err(format!(
                    "{}: readback differs at/after {:08X}",
                    image.name,
                    image.physical as usize + offset
                ));
            }
            offset += bytes.len();
            let percent = offset * 100 / image.bytes.len();
            if percent / 10 != progress / 10 || percent == 100 {
                println!("VERIFY {}: {percent}%", image.name);
                progress = percent;
            }
            Ok(())
        })?;
        println!("VERIFY OK: {} ({} bytes)", image.name, offset);
        Ok(())
    }
    pub fn format(&mut self, address: u32, length: u32) -> Result<()> {
        self.range(address, length)?;
        if address % 0x1000 != 0 || length % 0x1000 != 0 {
            return Err("Format range must be 4 KiB aligned".into());
        }
        self.wire.phase = "DA NOR format";
        // Original dispatcher 0xD4: storage, unused, validation, address, length.
        self.wire.send(&[0xd4, 0, 0, 0])?;
        self.wire.put32(address)?;
        self.wire.put32(length)?;
        self.wire.expect(&[ACK, ACK])?;
        let mut previous = 0;
        loop {
            let status = self.wire.be32()?;
            let progress = self.wire.byte()?;
            let expected_status = if progress == 100 { 0 } else { 0x0bcd };
            if status != expected_status || progress > 100 || progress < previous {
                return Err(format!(
                    "Format failed: status={status:08X}, progress={progress}"
                ));
            }
            if progress / 5 != previous / 5 || progress == 100 {
                println!("FORMAT {progress}%");
            }
            previous = progress;
            self.wire.send(&[ACK])?;
            if progress == 100 {
                break;
            }
        }
        self.wire.ack()?;
        Ok(())
    }
    fn packet(&mut self, bytes: &[u8]) -> Result<()> {
        for _ in 0..4 {
            self.wire.send(&[ACK])?;
            self.wire.send(bytes)?;
            self.wire.send(&sum16(bytes).to_be_bytes())?;
            match self.wire.byte()? {
                CONTINUE => return Ok(()),
                NACK => {
                    self.wire.ack()?;
                    self.wire.send(&[CONTINUE])?;
                }
                other => return Err(format!("{}: packet response {other:02X}", self.wire.phase)),
            }
        }
        Err("DA packet retry limit reached; transfer stopped".into())
    }
    pub fn application(&mut self, image: &Image) -> Result<()> {
        if image.kind != 0x100 {
            return Err("Legacy DA_MEM is only used for the main application".into());
        }
        self.wire.phase = "DA application memory plan";
        self.wire.send(&[0xd3, NACK, 1])?; // no FOTA, one main image
        self.wire.put32(image.physical)?;
        self.wire
            .put32(image.physical + image.bytes.len() as u32 - 1)?;
        self.wire.put32(image.kind as u32)?;
        self.wire.ack()?;
        if self.wire.byte()? != 1 {
            return Err("DA file count mismatch".into());
        }
        match self.wire.byte()? {
            ACK => {
                let erase_acks = self.wire.be32()?;
                if erase_acks > 8192 {
                    return Err("Invalid pre-erase count".into());
                }
                println!("DA pre-erase: {erase_acks} acknowledgements");
                for _ in 0..erase_acks {
                    self.wire.ack()?;
                }
            }
            NACK => {} // this NOR does not need the old asynchronous pre-erase
            other => return Err(format!("Unexpected pre-erase mode {other:02X}")),
        }
        self.wire.ack()?;
        self.wire.phase = "DA application download";
        self.wire.send(&[0xd5])?;
        self.wire.ack()?;
        self.wire.send(&[0])?;
        self.wire.put32(BLOCK as u32)?;
        self.wire.expect(&[ACK, ACK])?;
        self.download_packets(image)?;
        self.wire.expect(&[ACK, ACK, ACK])?;
        self.wire.send(&sum16(&image.bytes).to_be_bytes())?;
        self.wire.ack()?;
        println!("WRITE OK: {}; starting readback verification", image.name);
        Ok(())
    }
    fn download_packets(&mut self, image: &Image) -> Result<()> {
        let mut last = 101;
        for (index, chunk) in image.bytes.chunks(BLOCK).enumerate() {
            self.packet(chunk)?;
            let percent = ((index * BLOCK + chunk.len()) * 100 / image.bytes.len()) as u32;
            if percent / 5 != last / 5 || percent == 100 {
                println!("DOWNLOAD {}: {percent}%", image.name);
                last = percent;
            }
        }
        Ok(())
    }
    pub fn bootloaders(&mut self, images: &[Image]) -> Result<()> {
        if !matches!(images, [first] if first.kind == 1)
            && !matches!(images, [first, second] if first.kind == 1 && second.kind == 2)
        {
            return Err("Expected one ARM_BL, or original BL and EXT_BL".into());
        }
        self.wire.phase = "DA boot region metadata";
        // Use DA 0x51, not DA_MEM: BL's 70006000 is a RAM address and its real
        // flash location is 00000800. The original DA creates SF_BOOT/BRLYT.
        self.wire.send(&[0x51])?;
        self.wire.ack()?;
        self.wire.put32(BLOCK as u32)?;
        self.wire.send(&[7])?;
        self.wire.ack()?;
        let layout = self.wire.byte()?;
        if ![ACK, CONTINUE].contains(&layout) {
            return Err(format!("Boot layout response {layout:02X}"));
        }
        self.wire.put32(images.len() as u32)?;
        self.wire.send(&[ACK, NACK])?;
        self.wire.expect(&[ACK, ACK])?;
        for image in images {
            self.wire.send(&image.boot_descriptor())?;
            self.wire.ack()?;
        }
        self.wire.expect(&[ACK, ACK])?;
        self.wire.phase = "DA bootloader download";
        for image in images {
            self.download_packets(image)?;
        }
        self.wire.ack()?;
        let flags = self.wire.be32()?;
        println!("BOOT REGION DONE: flags={flags:08X}");
        Ok(())
    }
    pub fn verify_boot_metadata(&mut self, images: &[Image]) -> Result<()> {
        let mut metadata = Vec::with_capacity(1024);
        self.read(0, 1024, |bytes| {
            metadata.extend_from_slice(bytes);
            Ok(())
        })?;
        crate::images::validate_boot_metadata(&metadata, images)?;
        println!("BOOT METADATA OK: SF_BOOT/BRLYT; attributes match GFH_BL_INFO");
        Ok(())
    }
    pub fn reset(&mut self) -> Result<()> {
        self.wire.phase = "DA reset";
        self.wire.send(&[0xb9])?;
        self.wire.ack()?;
        self.wire.send(&[0xc9, 0])?;
        self.wire.ack()?;
        self.wire.send(&[0xdb, 1, 0x40, 0, 0, 0, 0])?;
        self.wire.ack()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::tests::Script;
    fn ready(bytes: &[u8]) -> DownloadAgent<Script> {
        DownloadAgent::from_ready(Wire::new(Script::new(bytes)), crate::images::FLASH_BYTES)
    }
    #[test]
    fn initialization_consumes_exact_original_info_and_sram_ready_trailers() {
        let mut input = vec![0xc0, 3, 2, 0x8e, ACK, ACK, NACK, CONTINUE];
        input.extend(0x0ca2u32.to_be_bytes());
        let mut info = [0; 188];
        info[8..12].copy_from_slice(&0x800000u32.to_be_bytes());
        info[187] = ACK;
        input.extend(info);
        input.push(ACK);
        input.extend(0u32.to_be_bytes());
        input.extend(0x407u32.to_be_bytes());
        input.extend([0xc1, NACK, 0xee]);
        let mut da = DownloadAgent::initialize(Wire::new(Script::new(&input))).unwrap();
        assert_eq!(
            da.wire.byte().unwrap(),
            0xee,
            "Initialization must leave the next command untouched"
        );
        assert_eq!(
            &da.wire.transport.output[..15],
            &[ACK, 5, 0xfe, 0, 8, 0, 0x70, 7, 0xff, 0xff, 2, 0, 0, 1, 8]
        );
        assert!(da.wire.transport.output.ends_with(&[0, 0, 0, 0, 0x59]));
    }
    #[test]
    fn unaligned_read_is_expanded_then_trimmed_and_checksum_checked_before_delivery() {
        let mut input = vec![ACK];
        let mut packet = vec![0; BLOCK];
        packet[0x100..0x103].copy_from_slice(&[1, 2, 3]);
        input.extend(&packet);
        input.extend(6u16.to_be_bytes());
        let mut da = ready(&input);
        let mut output: Vec<u8> = Vec::new();
        da.read(0x100, 3, |bytes| {
            output.extend(bytes);
            Ok(())
        })
        .unwrap();
        assert_eq!(output, [1, 2, 3]);
        let mut expected = vec![0xd6, 0];
        for n in [0u32, 4096, 4096] {
            expected.extend(n.to_be_bytes());
        }
        expected.push(ACK);
        assert_eq!(da.wire.transport.output, expected);
        let mut bad = input;
        *bad.last_mut().unwrap() = 7;
        let mut da = ready(&bad);
        assert!(
            da.read(0x100, 3, |_| panic!("Corrupt data was delivered"))
                .is_err()
        );
    }
    #[test]
    fn real_image_4000_byte_tail_is_read_as_full_block_not_the_broken_4066_byte_reply() {
        // The hardware report was 1,269,664 bytes: 309 full blocks + 4000.
        // In sub_100256C6 the USB loop sends floor(4000 / 64) * 64 bytes,
        // adds 4096 - 4000 padding bytes and a checksum: exactly 4066 bytes.
        let tail = 1_269_664 % BLOCK;
        assert_eq!(tail, 4000);
        assert_eq!(tail / 64 * 64 + BLOCK - tail + 2, 4066);
        let packet: Vec<u8> = (0..BLOCK).map(|i| (i % 251) as u8).collect();
        let mut input = vec![ACK];
        input.extend(&packet);
        input.extend(sum16(&packet).to_be_bytes());
        input.push(0xee);
        let mut da = ready(&input);
        let mut output = Vec::new();
        da.read(0x50000 + 309 * BLOCK as u32, tail as u32, |bytes| {
            output.extend_from_slice(bytes);
            Ok(())
        })
        .unwrap();
        assert_eq!(output, packet[..4000]);
        assert_eq!(&da.wire.transport.output[6..10], &4096u32.to_be_bytes());
        assert_eq!(da.wire.byte().unwrap(), 0xee);
    }
    #[test]
    fn unaligned_read_at_flash_end_does_not_cross_the_device_boundary() {
        let packet = vec![0xa7; BLOCK];
        let mut input = vec![ACK];
        input.extend(&packet);
        input.extend(sum16(&packet).to_be_bytes());
        let mut da = ready(&input);
        let mut output = Vec::new();
        da.read(crate::images::FLASH_BYTES - 3, 3, |bytes| {
            output.extend_from_slice(bytes);
            Ok(())
        })
        .unwrap();
        assert_eq!(output, [0xa7; 3]);
        assert_eq!(
            &da.wire.transport.output[2..6],
            &(crate::images::FLASH_BYTES - BLOCK as u32).to_be_bytes()
        );
        assert_eq!(&da.wire.transport.output[6..10], &4096u32.to_be_bytes());
    }
    #[test]
    fn busy_packet_is_retried_without_advancing_or_duplicating_checksum() {
        let mut da = ready(&[NACK, ACK, CONTINUE]);
        da.packet(&[1, 2]).unwrap();
        assert_eq!(
            da.wire.transport.output,
            [ACK, 1, 2, 0, 3, CONTINUE, ACK, 1, 2, 0, 3]
        );
    }
    #[test]
    fn application_preerase_modes_do_not_desynchronize_write_or_final_checksum() {
        for preerase in [false, true] {
            let mut input = vec![ACK, 1];
            if preerase {
                input.push(ACK);
                input.extend(2u32.to_be_bytes());
                input.extend([ACK, ACK]);
            } else {
                input.push(NACK);
            }
            input.extend([ACK, ACK, ACK, ACK, CONTINUE, ACK, ACK, ACK, ACK]);
            let mut da = ready(&input);
            let image = Image {
                boot_attributes: 0,
                name: "test".into(),
                bytes: vec![1, 2, 3, 4],
                kind: 0x100,
                load: 0x10050000,
                physical: 0x50000,
                content: 0,
                jump: 0,
                attributes: 3,
            };
            da.application(&image).unwrap();
            assert!(da.wire.transport.input.is_empty());
            let mut expected = vec![0xd3, NACK, 1];
            for n in [0x50000u32, 0x50003, 0x100] {
                expected.extend(n.to_be_bytes());
            }
            expected.extend([0xd5, 0]);
            expected.extend(4096u32.to_be_bytes());
            expected.extend([ACK, 1, 2, 3, 4, 0, 10, 0, 10]);
            assert_eq!(da.wire.transport.output, expected);
        }
    }
    #[test]
    fn format_accepts_progress_init_status_and_rejects_error_or_unaligned_ranges() {
        let mut input = vec![ACK, ACK];
        input.extend(0xbcdu32.to_be_bytes());
        input.push(50);
        input.extend(0u32.to_be_bytes());
        input.extend([100, ACK]);
        let mut da = ready(&input);
        da.format(0x500000, 0x60000).unwrap();
        assert!(da.wire.transport.input.is_empty());
        let mut da = ready(&[]);
        assert!(da.format(1, 4096).is_err());
        assert!(da.wire.transport.output.is_empty());
    }
    #[test]
    fn boot_download_uses_original_special_command_not_ram_address_as_flash_offset() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let images: Vec<_> = [
            "EKD01_CAN_BF_BOOTLOADER_V005_MT2502_EKD01_CAN_BF_N22.bin",
            "EXT_BOOTLOADER",
        ]
        .into_iter()
        .map(|name| {
            Image::parse(
                name.into(),
                std::fs::read(root.join("firmware/original").join(name)).unwrap(),
            )
            .unwrap()
        })
        .collect();
        for count in [1, 2] {
            let images = &images[..count];
            let mut input = vec![ACK, ACK, CONTINUE];
            input.extend(std::iter::repeat_n(ACK, 4 + count));
            for image in images {
                input.extend(std::iter::repeat_n(
                    CONTINUE,
                    image.bytes.len().div_ceil(BLOCK),
                ));
            }
            input.push(ACK);
            input.extend(2u32.to_be_bytes());
            let mut da = ready(&input);
            da.bootloaders(images).unwrap();
            assert!(da.wire.transport.input.is_empty());
            assert_eq!(da.wire.transport.output[0], 0x51);
            assert_eq!(images[0].physical, 0x800);
            assert_eq!(
                &da.wire.transport.output[6..10],
                &(count as u32).to_be_bytes()
            );
            if count == 2 {
                assert_eq!(images[1].physical, 0x27000);
            }
        }
    }
}
