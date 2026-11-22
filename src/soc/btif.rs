use super::{DebugUart, mmio::*, registers::*, ring::RxRing, time::Timer};
static RX: RxRing<8192> = RxRing::new();
const IRQ_BIT: u32 = 1 << (EKD_IRQ_BTIF_LINE - 32);
#[derive(Clone, Copy, Debug)]
pub enum BtifError {
    Offline,
    Timeout,
    Overflow,
    Packet,
}
impl core::fmt::Display for BtifError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "{self:?}")
    }
}
impl core::error::Error for BtifError {}
impl From<bt_hci_transport::ReadHciError<core::convert::Infallible>> for BtifError {
    fn from(_: bt_hci_transport::ReadHciError<core::convert::Infallible>) -> Self {
        Self::Packet
    }
}
impl embedded_io::Error for BtifError {
    fn kind(&self) -> embedded_io::ErrorKind {
        embedded_io::ErrorKind::Other
    }
}

pub struct Btif {
    opened: bool,
}
impl Btif {
    pub async fn open() -> Result<Self, BtifError> {
        RX.clear();
        write32(IRQ_MASK_SET1, IRQ_BIT);
        write32(DRVPDN_CON0_CLR, EKD_PDN_BT_CLOCKS);
        update32(EKD_BT_POWER_CON1, 0, EKD_BT_POWER_ON);
        embassy_time::Timer::after_ticks(1).await;
        write32(DRVPDN_CON2_CLR, EKD_PDN_BTIF);
        embassy_time::Timer::after_ticks(1).await;
        write16(BTIF_IER_ADDR, 0);
        write16(BTIF_DMA_EN_ADDR, 0);
        write16(BTIF_SLEEP_EN_ADDR, 0);
        let mut btif = Self { opened: true };
        if read16(BTIF_LSR_ADDR) == 0xffff {
            btif.close().await;
            return Err(BtifError::Offline);
        }
        for _ in 0..256 {
            if read16(BTIF_LSR_ADDR) as u32 & BTIF_LSR_DR == 0 {
                break;
            }
            let _ = read16(BTIF_RBR_ADDR);
            let _ = read16(GPIO_DIR0);
        }
        if read16(BTIF_LSR_ADDR) as u32 & BTIF_LSR_DR != 0 {
            btif.close().await;
            return Err(BtifError::Offline);
        }
        write32(IRQ_SENS_SET1, IRQ_BIT);
        write32(IRQ_EOIOH1, IRQ_BIT);
        write16(BTIF_IER_ADDR, BTIF_IER_RXEN as u16);
        write32(IRQ_MASK_CLR1, IRQ_BIT);
        DebugUart::log("[BTIF] OPEN; internal FIFO IRQ43\n");
        Ok(btif)
    }
    pub async fn close(&mut self) {
        write32(IRQ_MASK_SET1, IRQ_BIT);
        if self.opened {
            write16(BTIF_IER_ADDR, 0);
            write32(IRQ_EOIOH1, IRQ_BIT);
            write32(DRVPDN_CON2_SET, EKD_PDN_BTIF);
            write32(DRVPDN_CON0_SET, EKD_PDN_BT_ARM7);
            embassy_time::Timer::after_ticks(1).await;
            write32(DRVPDN_CON0_SET, EKD_PDN_BT_BUS_26M);
            update32(EKD_BT_POWER_CON1, EKD_BT_POWER_OFF_MASK, 0);
        }
        self.opened = false;
    }
    pub fn irq() {
        for _ in 0..256 {
            let status = read16(BTIF_LSR_ADDR) as u32;
            if status & BTIF_LSR_DR == 0 {
                break;
            }
            let byte = read16(BTIF_RBR_ADDR) as u8;
            let _ = read16(GPIO_DIR0);
            if !RX.push_irq(byte) {
                write16(BTIF_IER_ADDR, 0);
                write32(IRQ_MASK_SET1, IRQ_BIT);
                break;
            }
        }
        let _ = read16(BTIF_IIR_ADDR);
        write32(IRQ_EOIOH1, IRQ_BIT);
    }
    pub async fn read(&self, bytes: &mut [u8]) -> Result<usize, BtifError> {
        if !self.opened {
            return Err(BtifError::Offline);
        }
        if RX.overruns() != 0 {
            return Err(BtifError::Overflow);
        }
        Ok(RX.read(bytes).await)
    }
    pub async fn send(&self, bytes: &[u8]) -> Result<(), BtifError> {
        if !self.opened {
            return Err(BtifError::Offline);
        }
        let started = Timer::now();
        for byte in bytes {
            loop {
                let mut ready = false;
                for _ in 0..4096 {
                    if read16(BTIF_LSR_ADDR) as u32 & BTIF_LSR_TEMT != 0 {
                        ready = true;
                        break;
                    }
                }
                if ready {
                    write16(BTIF_THR_ADDR, *byte as u16);
                    let _ = read16(GPIO_DIR0);
                    break;
                }
                if Timer::now().wrapping_sub(started) >= 128 {
                    return Err(BtifError::Timeout);
                }
                embassy_time::Timer::after_ticks(1).await;
            }
        }
        Ok(())
    }
}
