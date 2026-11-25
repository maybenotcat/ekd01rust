use crate::soc::btif::{Btif, BtifError};
use bt_hci::transport::{PacketKind, PacketToController, PacketToHost, Transport};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};

pub struct HciTransport {
    io: Btif,
    rx: Mutex<CriticalSectionRawMutex, ()>,
    tx: Mutex<CriticalSectionRawMutex, ()>,
    shared_acl: (u16, u8),
}
impl HciTransport {
    pub fn new(io: Btif) -> Self {
        Self {
            io,
            rx: Mutex::new(()),
            tx: Mutex::new(()),
            shared_acl: (0, 0),
        }
    }
    pub fn io(&self) -> &Btif {
        &self.io
    }
    pub async fn close(&mut self) {
        self.io.close().await;
    }
    pub fn set_shared_buffers(&mut self, bytes: u16, packets: u16) {
        self.shared_acl = (bytes, packets.min(255) as u8);
    }
    async fn exact(&self, out: &mut [u8]) -> Result<(), TransportError> {
        let mut used = 0;
        while used < out.len() {
            let count = self.io.read(&mut out[used..]).await?;
            if count == 0 {
                return Err(TransportError::packet());
            }
            used += count;
        }
        Ok(())
    }
}
struct Writer<'a>(&'a Btif);
impl embedded_io::ErrorType for Writer<'_> {
    type Error = TransportError;
}
impl embedded_io::ErrorType for HciTransport {
    type Error = TransportError;
}
impl embedded_io::ErrorType for &HciTransport {
    type Error = TransportError;
}
impl embedded_io_async::Write for Writer<'_> {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize, TransportError> {
        self.0.send(bytes).await?;
        Ok(bytes.len())
    }
    async fn flush(&mut self) -> Result<(), TransportError> {
        Ok(())
    }
}
impl Transport for HciTransport {
    async fn read<'a, P: PacketToHost<'a>>(
        &self,
        buffer: &'a mut [u8],
    ) -> Result<P, TransportError> {
        let _guard = self.rx.lock().await;
        let mut indicator = [0; 1];
        self.exact(&mut indicator).await?;
        let (kind, header) = match indicator[0] {
            4 => (PacketKind::Event, 2),
            2 => (PacketKind::AclData, 4),
            _ => {
                crate::soc::DebugUart::hex("[BT] INVALID_H4_INDICATOR", indicator[0] as u32);
                return Err(TransportError::packet());
            }
        };
        let mut raw = [0u8; 259];
        self.exact(&mut raw[..header]).await?;
        let length = if header == 2 {
            raw[1] as usize
        } else {
            u16::from_le_bytes([raw[2], raw[3]]) as usize
        };
        if header + length > raw.len() {
            return Err(TransportError::packet());
        }
        self.exact(&mut raw[header..header + length]).await?;
        if header == 2 && raw[0] == 0x0e && length >= 4 {
            crate::soc::DebugUart::hex(
                "[BT] HOST_CC_OPCODE_CREDIT_STATUS",
                ((u16::from_le_bytes([raw[3], raw[4]]) as u32) << 16)
                    | ((raw[2] as u32) << 8)
                    | raw[5] as u32,
            );
        }
        // Bluetooth Core 4.0, Vol 2 Part E 7.4.5: a dual-mode controller may
        // report zero dedicated LE buffers. Trouble expects nonzero LE credits;
        // substitute only the real shared ACL geometry queried before startup.
        if header == 2
            && length == 7
            && raw[0] == 0x0e
            && raw[3] == 2
            && raw[4] == 0x20
            && raw[5] == 0
            && raw[8] == 0
        {
            if self.shared_acl.0 == 0 || self.shared_acl.1 == 0 {
                return Err(TransportError::packet());
            }
            raw[6..8].copy_from_slice(&self.shared_acl.0.to_le_bytes());
            raw[8] = self.shared_acl.1;
            crate::soc::DebugUart::log("[BT] LE_USES_SHARED_ACL_BUFFERS\n");
        }
        let mut reader = PacketReader {
            bytes: &raw[..header + length],
            offset: 0,
        };
        P::read_hci(kind, &mut reader, buffer).map_err(|error| {
            use core::fmt::Write;
            let _ = writeln!(crate::soc::DebugUart, "[BT] HCI_PARSE_ERROR: {error:?}");
            TransportError::packet()
        })
    }
    async fn write<P: PacketToController>(&self, packet: &P) -> Result<(), TransportError> {
        let _guard = self.tx.lock().await;
        if P::KIND as u8 == PacketKind::Cmd as u8 {
            let mut bytes = [0u8; 258];
            let length = {
                let mut writer = PacketWriter {
                    bytes: &mut bytes,
                    offset: 0,
                };
                packet.write_hci(&mut writer)?;
                writer.offset
            };
            if length >= 3 {
                crate::soc::DebugUart::hex(
                    "[BT] HOST_TX_OPCODE",
                    u16::from_le_bytes([bytes[0], bytes[1]]) as u32,
                );
            }
            if length == 11 && bytes[2] == 8 {
                let opcode = u16::from_le_bytes([bytes[0], bytes[1]]);
                if opcode == 0x2001 {
                    // The observed controller is Bluetooth 4.0. Subscribe only
                    // to its five defined LE events, never post-4.0 reserved bits.
                    bytes[3..11].copy_from_slice(&0x1fu64.to_le_bytes());
                } else if opcode == 0x0c63 {
                    // Encryption Change v2 is not present on this controller.
                    bytes[3..11].fill(0);
                }
            }
            self.io.send(&[PacketKind::Cmd as u8]).await?;
            return self
                .io
                .send(&bytes[..length])
                .await
                .map_err(TransportError::from);
        }
        self.io.send(&[P::KIND as u8]).await?;
        packet.write_hci_async(Writer(&self.io)).await
    }
}
struct PacketWriter<'a> {
    bytes: &'a mut [u8],
    offset: usize,
}
impl embedded_io::ErrorType for PacketWriter<'_> {
    type Error = TransportError;
}
impl embedded_io::Write for PacketWriter<'_> {
    fn write(&mut self, data: &[u8]) -> Result<usize, TransportError> {
        if data.len() > self.bytes.len() - self.offset {
            return Err(TransportError::packet());
        }
        self.bytes[self.offset..self.offset + data.len()].copy_from_slice(data);
        self.offset += data.len();
        Ok(data.len())
    }
    fn flush(&mut self) -> Result<(), TransportError> {
        Ok(())
    }
}
struct PacketReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl embedded_io::ErrorType for PacketReader<'_> {
    type Error = TransportError;
}
impl embedded_io::Read for PacketReader<'_> {
    fn read(&mut self, out: &mut [u8]) -> Result<usize, TransportError> {
        let size = out.len().min(self.bytes.len() - self.offset);
        out[..size].copy_from_slice(&self.bytes[self.offset..self.offset + size]);
        self.offset += size;
        Ok(size)
    }
}
impl Transport for &HciTransport {
    async fn read<'a, P: PacketToHost<'a>>(
        &self,
        buffer: &'a mut [u8],
    ) -> Result<P, TransportError> {
        <HciTransport as Transport>::read(*self, buffer).await
    }
    async fn write<P: PacketToController>(&self, packet: &P) -> Result<(), TransportError> {
        <HciTransport as Transport>::write(*self, packet).await
    }
}

#[derive(Debug)]
pub struct TransportError(BtifError);
impl TransportError {
    fn packet() -> Self {
        Self(BtifError::Packet)
    }
}
impl From<BtifError> for TransportError {
    fn from(error: BtifError) -> Self {
        Self(error)
    }
}
impl core::fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(formatter)
    }
}
impl core::error::Error for TransportError {}
impl embedded_io::Error for TransportError {
    fn kind(&self) -> embedded_io::ErrorKind {
        embedded_io::ErrorKind::Other
    }
}
impl From<bt_hci_transport::ReadHciError<core::convert::Infallible>> for TransportError {
    fn from(_: bt_hci_transport::ReadHciError<core::convert::Infallible>) -> Self {
        Self::packet()
    }
}
