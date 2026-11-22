use super::patch::PAYLOAD as PATCH;
use super::{identity::Identity, transport::HciTransport};
use crate::{
    soc::{
        DebugUart,
        btif::{Btif, BtifError},
    },
    storage::FlashBus,
};
const CONFIG_BYTES: [u8; 47] = [
    0, 0x66, 0, 0, 0, 0, 0xff, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0x10, 0,
    0x60, 0, 0x23, 7, 0x80, 0, 6, 7, 7, 3, 0x40, 0x1f, 0x40, 0x1f, 0, 4, 0x80, 0, 0,
];
const OPCODES: [u16; 11] = [
    0xfc1a, 0xfc7f, 0xfc1b, 0xfc75, 0xfc76, 0x0c0a, 0x0c26, 0xfc72, 0xfc79, 0xfc7a, 0xfc7d,
];
const LENGTHS: [usize; 11] = [6, 1, 1, 16, 3, 1, 2, 1, 6, 7, 2];

#[derive(Debug)]
pub enum ControllerError {
    Transport(BtifError),
    Status(u16, u8),
    Deadline,
    Hardware(u16),
    Response,
    Identity,
}
impl From<BtifError> for ControllerError {
    fn from(error: BtifError) -> Self {
        Self::Transport(error)
    }
}
pub struct BluetoothController {
    pub transport: HciTransport,
    pub address: [u8; 6],
}
impl BluetoothController {
    pub async fn start(flash: FlashBus) -> Result<Self, ControllerError> {
        DebugUart::log("[BT] CONTROLLER_INIT_ENTER\n");
        let io = Btif::open().await?;
        let mut controller = Self {
            transport: HciTransport::new(io),
            address: [0; 6],
        };
        if let Err(error) = controller.configure(flash).await {
            controller.transport.close().await;
            return Err(error);
        }
        Ok(controller)
    }
    async fn exact(&self, out: &mut [u8]) -> Result<(), ControllerError> {
        let mut used = 0;
        while used < out.len() {
            let size = self.transport.io().read(&mut out[used..]).await?;
            if size == 0 {
                return Err(ControllerError::Response);
            }
            used += size;
        }
        Ok(())
    }
    async fn command(
        &mut self,
        opcode: u16,
        payload: &[u8],
        reply: &mut [u8],
    ) -> Result<usize, ControllerError> {
        if payload.len() > 255 {
            return Err(ControllerError::Response);
        }
        if opcode != 0xfcc4 {
            DebugUart::hex("[BT] TX_OPCODE", opcode as u32);
        }
        self.transport
            .io()
            .send(&[1, opcode as u8, (opcode >> 8) as u8, payload.len() as u8])
            .await?;
        self.transport.io().send(payload).await?;
        embassy_time::with_timeout(embassy_time::Duration::from_secs(3), async {
            loop {
                let mut header = [0; 3];
                self.exact(&mut header).await?;
                if header[0] != 4 {
                    return Err(ControllerError::Response);
                }
                let mut body = [0; 255];
                let size = header[2] as usize;
                self.exact(&mut body[..size]).await?;
                if header[1] != 0x0e || size < 4 || u16::from_le_bytes([body[1], body[2]]) != opcode
                {
                    continue;
                }
                if body[3] != 0 {
                    return Err(ControllerError::Status(opcode, body[3]));
                }
                let count = size - 4;
                if count > reply.len() {
                    return Err(ControllerError::Response);
                }
                reply[..count].copy_from_slice(&body[4..size]);
                return Ok(count);
            }
        })
        .await
        .map_err(|_| ControllerError::Deadline)?
    }
    async fn configure(&mut self, flash: FlashBus) -> Result<(), ControllerError> {
        let mut reply = [0; 249];
        self.command(0x0c03, &[], &mut reply).await?;
        self.command(0x1001, &[], &mut reply).await?;
        self.command(0x0c14, &[], &mut reply).await?;
        if self.command(0xfcd1, &[0, 0, 0, 3], &mut reply).await? < 2 {
            return Err(ControllerError::Response);
        }
        let hardware = u16::from_le_bytes([reply[0], reply[1]]);
        DebugUart::hex("[BT] HW_ID", hardware as u32);
        if hardware != 0x8a00 {
            return Err(ControllerError::Hardware(hardware));
        }
        if self.command(0xfcd1, &[4, 0, 0, 3], &mut reply).await? < 1 {
            return Err(ControllerError::Response);
        }
        DebugUart::hex("[BT] ROM_VERSION", reply[0] as u32);
        if reply[0] == 0 {
            let mut offset = 0;
            for chunk in PATCH.chunks(240) {
                let mut packet = [0; 245];
                packet[0] = if offset + chunk.len() == PATCH.len() {
                    2
                } else if offset == 0 {
                    0
                } else {
                    1
                };
                packet[1] = chunk.len() as u8;
                packet[5..5 + chunk.len()].copy_from_slice(chunk);
                self.command(0xfcc4, &packet[..5 + chunk.len()], &mut reply)
                    .await?;
                offset += chunk.len();
                if offset == chunk.len() || offset == PATCH.len() {
                    DebugUart::hex("[BT] PATCH_BYTES_SENT", offset as u32);
                }
            }
        }
        self.command(0x0c03, &[], &mut reply).await?;
        if self.command(0x2018, &[], &mut reply).await? < 8
            || reply[..6].iter().all(|b| *b == 0)
            || reply[..6].iter().all(|b| *b == 255)
        {
            return Err(ControllerError::Response);
        }
        self.address =
            Identity::load_or_create(&mut flash.borrow_mut(), reply[..6].try_into().unwrap())
                .map_err(|_| ControllerError::Identity)?;
        let mut offset = 0;
        for (index, (&opcode, &length)) in OPCODES.iter().zip(LENGTHS.iter()).enumerate() {
            let mut data = [0u8; 16];
            data[..length].copy_from_slice(&CONFIG_BYTES[offset..offset + length]);
            offset += length;
            if index == 0 {
                data[..6].copy_from_slice(&self.address);
            }
            if index == 9 {
                data[0] = 0;
                data[3] = 0;
                data[4] = 0;
            }
            self.command(opcode, &data[..length], &mut reply).await?;
        }
        self.command(
            0xfc1e,
            &[0xbf, 0x3e, 0x8d, 0xfe, 0x9b, 0xff, 0x59, 0x87],
            &mut reply,
        )
        .await?;
        self.command(
            0xfc7c,
            &[1, 0xfa, 0x0a, 2, 0, 0xa6, 0x0e, 0, 0x40, 0],
            &mut reply,
        )
        .await?;
        self.command(0xfcd2, &[0], &mut reply).await?;
        self.command(0x0c03, &[], &mut reply).await?;
        if self.command(0x1005, &[], &mut reply).await? < 7 {
            return Err(ControllerError::Response);
        }
        self.transport.set_shared_buffers(
            u16::from_le_bytes([reply[0], reply[1]]),
            u16::from_le_bytes([reply[3], reply[4]]),
        );
        DebugUart::log("[BT] CONTROLLER_CONFIG_COMPLETE; sleep disabled\n");
        Ok(())
    }
}
