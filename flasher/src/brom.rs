//! BootROM handshake and original DA upload. No flash write commands here.
use crate::{
    Result, agent,
    wire::{Wire, xor16},
};
use std::{
    io::{Read, Write},
    time::{Duration, Instant},
};

pub struct Chip {
    pub hardware: u16,
    pub software: u16,
    pub code: u16,
    pub subcode: u16,
}
impl Chip {
    pub fn print(&self) {
        println!(
            "BROM: code={:04X} sub={:04X} HW={:04X} SW={:04X}",
            self.code, self.subcode, self.hardware, self.software
        );
    }
}

pub struct BootRom<T> {
    pub wire: Wire<T>,
}
impl<T: Read + Write> BootRom<T> {
    pub fn connect(transport: T, timeout: Duration) -> Result<Self> {
        let mut wire = Wire::new(transport);
        wire.phase = "BROM handshake";
        wire.timeout = Duration::from_millis(80);
        let start = Instant::now();
        loop {
            if start.elapsed() >= timeout {
                return Err(
                    "BootROM handshake timed out; reconnect USB with the program waiting".into(),
                );
            }
            wire.send(&[0xa0])?;
            if matches!(wire.byte(), Ok(0x5f)) {
                break;
            }
        }
        wire.timeout = Duration::from_secs(15);
        // Synchronization bytes are deliberately sent together, as in the reference.
        wire.send(&[0x0a, 0x50, 0x05])?;
        wire.expect(&[0xf5, 0xaf, 0xfa])?;
        Ok(Self { wire })
    }
    fn read_legacy16(&mut self, address: u32) -> Result<u16> {
        self.wire.echo(&[0xa2])?;
        self.wire.echo(&address.to_be_bytes())?;
        self.wire.echo(&1u32.to_be_bytes())?;
        self.wire.be16()
    }
    pub fn chip(&mut self) -> Result<Chip> {
        self.wire.phase = "BROM chip identification";
        Ok(Chip {
            hardware: self.read_legacy16(0x80000000)?,
            software: self.read_legacy16(0x80000004)?,
            code: self.read_legacy16(0x80000008)?,
            subcode: self.read_legacy16(0x8000000c)?,
        })
    }
    fn write(&mut self, command: u8, address: u32, value: &[u8]) -> Result<()> {
        self.wire.echo(&[command])?;
        self.wire.echo(&address.to_be_bytes())?;
        self.wire.echo(&1u32.to_be_bytes())?;
        self.wire.expect(&[0, 1])?;
        self.wire.echo(value)?;
        self.wire.expect(&[0, 1])
    }
    pub fn start_agent(mut self) -> Result<Wire<T>> {
        agent::validate()?;
        self.wire.phase = "BROM power/watchdog setup";
        // Reference MT6261 host setup: PMU/RTC power latch, watchdog and remap.
        // These are SoC registers; the auxiliary UART3 MCU is not accessed.
        for (address, value) in [
            (0xa0700a28, 0x4010u16),
            (0xa0700a00, 0xf210),
            (0xa0030000, 0x2200),
            (0xa071004c, 0x1a57),
            (0xa071004c, 0x2b68),
            (0xa071004c, 0x042e),
            (0xa0710068, 0x586a),
            (0xa0710074, 1),
            (0xa0710068, 0x9136),
            (0xa0710074, 1),
            (0xa0710000, 0x430e),
            (0xa0710074, 1),
        ] {
            self.write(0xd2, address, &value.to_be_bytes())?;
        }
        self.write(0xd4, 0xa0510000, &2u32.to_be_bytes())?;
        for stage in &agent::STAGES {
            self.wire.phase = "BROM SEND_DA";
            println!(
                "DA -> {:08X}: {} bytes (original binary)",
                stage.address,
                stage.bytes.len()
            );
            self.wire.echo(&[0xd7])?;
            self.wire.echo(&stage.address.to_be_bytes())?;
            self.wire.echo(&(stage.bytes.len() as u32).to_be_bytes())?;
            self.wire.echo(&stage.signature_bytes.to_be_bytes())?;
            self.wire.expect(&[0, 0])?;
            for chunk in stage.bytes.chunks(1024) {
                self.wire.send(chunk)?;
            }
            let checksum = self.wire.be16()?;
            if checksum != xor16(stage.bytes) {
                return Err(format!("DA checksum mismatch at {:08X}", stage.address));
            }
            self.wire.expect(&[0, 0])?;
        }
        self.wire.phase = "BROM JUMP_DA";
        self.wire.echo(&[0xd5])?;
        self.wire.echo(&agent::STAGES[0].address.to_be_bytes())?;
        self.wire.expect(&[0, 0])?;
        Ok(self.wire)
    }
}
