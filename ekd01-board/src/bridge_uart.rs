//! N22 auxiliary MCU link configuration; MCU firmware/reset are not touched.
use core::marker::PhantomData;
use mt2502_hal::{
    environment::Trace,
    uart3::{RateConfiguration, Uart3},
};
pub struct BridgeUart<P: Trace> {
    uart: Uart3,
    _environment: PhantomData<P>,
}
impl<P: Trace> BridgeUart<P> {
    pub fn initialize() -> Self {
        let uart = Uart3::initialize(
            crate::pins::MCU_RX,
            crate::pins::MCU_TX,
            RateConfiguration {
                fix: 15,
                fraction_l: 0x55,
                fraction_m: 0,
                step: 3,
                divisor_l: 1,
                divisor_h: 0,
                step_count: 55,
                sample_count: 27,
            },
        );
        P::log("[BRIDGE] UART3 230400; IRQ29; MCU retained\n");
        Self {
            uart,
            _environment: PhantomData,
        }
    }
    pub fn irq() {
        Uart3::irq();
    }
    pub fn receive(&mut self) -> Option<u8> {
        self.uart.receive()
    }
    pub fn send(&mut self, bytes: &[u8]) -> bool {
        self.uart.send(bytes)
    }
    pub fn errors(&self) -> u32 {
        self.uart.errors()
    }
    pub fn stop(&mut self) {
        self.uart.stop();
    }
}
