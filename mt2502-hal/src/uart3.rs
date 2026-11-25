use super::{gpio::Pin, mmio::*, registers::*, ring::RxRing};
use crate::environment::Trace;
use core::marker::PhantomData;

// Embassy rendering is cooperative. Buffer beyond one C task timeslice without
// dropping a burst while Slint finishes a synchronous raster operation.
static RX: RxRing<8192> = RxRing::new();
pub struct Uart3<P: Trace> {
    _environment: PhantomData<P>,
    _rx: Pin,
    _tx: Pin,
}
impl<P: Trace> Uart3<P> {
    pub fn initialize(rx_pin: u32, tx_pin: u32) -> Self {
        let mut rx = unsafe { Pin::claim(rx_pin) };
        let mut tx = unsafe { Pin::claim(tx_pin) };
        critical_section::with(|_| {
            write32(IRQ_MASK_SET0, 1 << EKD_IRQ_UART3_LINE);
            write32(DRVPDN_CON1_CLR, EKD_PDN_UART3);
            tx.set(true);
            rx.input();
            tx.output();
            rx.mode(3);
            tx.mode(3);
            let b = UART3_base;
            for (address, value) in [
                (UART_LCR(b), UART_LCR_8N1),
                (UART_IER(b), 0),
                (UART_SLEEP_EN(b), 0),
                (UART_RXDMA(b), 0),
                (UART_AUTOBAUD_EN(b), 0),
                (UART_LCR(b), UART_LCR_ENHANCED_ACCESS),
                (UART_EFR(b), UART_EFR_EN),
                (UART_LCR(b), UART_LCR_8N1),
                (UART_MCR(b), 0),
                (UART_ESCAPE_EN(b), 0),
                (UART_RATE_FIX_REG(b), 15),
                (UART_FRACDIV_L(b), 0x55),
                (UART_FRACDIV_M(b), 0),
                (UART_RATE_STEP(b), 3),
                (UART_LCR(b), UART_LCR_DLAB | UART_LCR_8N1),
                (UART_DLL(b), 1),
                (UART_DLH(b), 0),
                (UART_STEP_COUNT(b), 55),
                (UART_SAMPLE_COUNT(b), 27),
                (UART_LCR(b), UART_LCR_8N1),
                (UART_FCR(b), UART_FCR_FIFOINI),
            ] {
                write16(address, value as u16);
            }
            RX.clear();
            write32(IRQ_SENS_SET0, 1 << EKD_IRQ_UART3_LINE);
            write32(IRQ_EOIOH0, 1 << EKD_IRQ_UART3_LINE);
            write16(UART_IER(b), (UART_IER_ERBFI | UART_IER_ELSI) as u16);
            write32(IRQ_MASK_CLR0, 1 << EKD_IRQ_UART3_LINE);
        });
        P::log("[BRIDGE] UART3 230400; IRQ29; MCU retained\n");
        Self {
            _environment: PhantomData,
            _rx: rx,
            _tx: tx,
        }
    }
    pub fn irq() {
        for _ in 0..128 {
            let status = read16(UART_LSR(UART3_base)) as u32;
            if status & UART_LSR_ERROR_MASK != 0 {
                RX.note_error();
            }
            if status & UART_LSR_DR == 0 {
                break;
            }
            RX.push_irq(read16(UART_RBR(UART3_base)) as u8);
        }
        let _ = read16(UART_IIR(UART3_base));
        write32(IRQ_EOIOH0, 1 << EKD_IRQ_UART3_LINE);
        let _ = read32(IRQ_MASK0);
    }
    pub fn receive(&mut self) -> Option<u8> {
        RX.pop()
    }
    pub fn send(&mut self, bytes: &[u8]) -> bool {
        if bytes.is_empty()
            || bytes.len() > 15
            || read16(UART_LSR(UART3_base)) as u32 & UART_LSR_THRE == 0
        {
            return false;
        }
        for byte in bytes {
            write16(UART_THR(UART3_base), *byte as u16);
        }
        true
    }
    pub fn errors(&self) -> u32 {
        RX.overruns()
    }
    pub fn stop(&mut self) {
        critical_section::with(|_| {
            write16(UART_IER(UART3_base), 0);
            write32(IRQ_MASK_SET0, 1 << EKD_IRQ_UART3_LINE);
        });
    }
}
