//! Bounded, exact transfers. Serial reads may return only part of a response.
use crate::Result;
use std::{
    io::{self, Read, Write},
    time::{Duration, Instant},
};

pub const ACK: u8 = 0x5a;
pub const NACK: u8 = 0xa5;
pub const CONTINUE: u8 = 0x69;

pub struct Wire<T> {
    pub transport: T,
    pub timeout: Duration,
    pub phase: &'static str,
}
impl<T: Read + Write> Wire<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            timeout: Duration::from_secs(15),
            phase: "connect",
        }
    }
    pub fn send(&mut self, bytes: &[u8]) -> Result<()> {
        self.transport
            .write_all(bytes)
            .map_err(|error| format!("{}: write: {error}", self.phase))
    }
    pub fn receive(&mut self, count: usize) -> Result<Vec<u8>> {
        let mut output = vec![0; count];
        let mut offset = 0;
        let start = Instant::now();
        while offset < count {
            if start.elapsed() >= self.timeout {
                return Err(format!(
                    "{}: timeout after {offset}/{count} bytes",
                    self.phase
                ));
            }
            match self.transport.read(&mut output[offset..]) {
                Ok(0) => {
                    return Err(format!(
                        "{}: port closed after {offset}/{count} bytes",
                        self.phase
                    ));
                }
                Ok(n) => offset += n,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::Interrupted
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::WouldBlock
                    ) => {}
                Err(error) => {
                    return Err(format!(
                        "{}: read after {offset}/{count} bytes: {error}",
                        self.phase
                    ));
                }
            }
        }
        Ok(output)
    }
    pub fn byte(&mut self) -> Result<u8> {
        Ok(self.receive(1)?[0])
    }
    pub fn be16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.receive(2)?.try_into().unwrap()))
    }
    pub fn be32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.receive(4)?.try_into().unwrap()))
    }
    pub fn put32(&mut self, value: u32) -> Result<()> {
        self.send(&value.to_be_bytes())
    }
    pub fn expect(&mut self, wanted: &[u8]) -> Result<()> {
        let actual = self.receive(wanted.len())?;
        if actual != wanted {
            return Err(format!(
                "{}: expected {wanted:02X?}, received {actual:02X?}",
                self.phase
            ));
        }
        Ok(())
    }
    // BROM echoes the command and each argument before returning a status.
    pub fn echo(&mut self, bytes: &[u8]) -> Result<()> {
        self.send(bytes)?;
        self.expect(bytes)
    }
    pub fn ack(&mut self) -> Result<()> {
        self.expect(&[ACK])
    }
}
pub fn sum16(bytes: &[u8]) -> u16 {
    bytes
        .iter()
        .fold(0u16, |sum, byte| sum.wrapping_add(*byte as u16))
}
pub fn xor16(bytes: &[u8]) -> u16 {
    bytes.chunks(2).fold(0, |sum, pair| {
        sum ^ (pair[0] as u16 | ((pair.get(1).copied().unwrap_or(0) as u16) << 8))
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    #[derive(Default)]
    pub struct Script {
        pub input: std::collections::VecDeque<u8>,
        pub output: Vec<u8>,
    }
    impl Script {
        pub fn new(bytes: &[u8]) -> Self {
            Self {
                input: bytes.iter().copied().collect(),
                output: Vec::new(),
            }
        }
    }
    impl Read for Script {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if let Some(byte) = self.input.pop_front() {
                output[0] = byte;
                Ok(1)
            } else {
                Ok(0)
            }
        }
    }
    impl Write for Script {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.output.extend(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn fragmented_reads_and_bad_echo_are_not_silently_accepted() {
        let mut wire = Wire::new(Script::new(&[1, 2, 3, 4, ACK]));
        assert_eq!(wire.be32().unwrap(), 0x01020304);
        wire.ack().unwrap();
        assert!(wire.echo(&[0xd5]).is_err());
        assert_eq!(sum16(&[255; 258]), 254);
        assert_eq!(xor16(&[1, 2, 3]), 0x0202);
    }
}
