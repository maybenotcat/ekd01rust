use super::{DebugUart, mmio::*, registers::*};
use core::{
    cell::{Cell, RefCell},
    task::Waker,
};
use critical_section::Mutex;
use embassy_time_driver::Driver;
use embassy_time_queue_utils::queue_generic::ConstGenericQueue;

struct GptTime {
    ticks: Mutex<Cell<u64>>,
    timers: Mutex<RefCell<ConstGenericQueue<32>>>,
}
embassy_time_driver::time_driver_impl!(static TIME: GptTime = GptTime {
    ticks: Mutex::new(Cell::new(0)),
    timers: Mutex::new(RefCell::new(ConstGenericQueue::new())),
});
impl Driver for GptTime {
    fn now(&self) -> u64 {
        critical_section::with(|cs| self.ticks.borrow(cs).get())
    }
    fn schedule_wake(&self, at: u64, waker: &Waker) {
        critical_section::with(|cs| {
            self.timers.borrow(cs).borrow_mut().schedule_wake(at, waker);
        });
    }
}

pub struct Timer;
impl Timer {
    pub fn initialize_interrupts() {
        write32(IRQ_MASK_SET0, u32::MAX);
        write32(IRQ_MASK_SET1, u32::MAX);
        let _ = read32(IRQ_MASK0);
        for index in 0..5 {
            write32(IRQ_SEL(index), EKD_IRQ_PATCH_DISABLED);
        }
        write32(IRQ_FSEL0, 0);
        write32(IRQ_FSEL1, 0);
        write32(IRQ_SOFT_CLR0, u32::MAX);
        write32(IRQ_SOFT_CLR1, u32::MAX);
        write16(GPT1_CTRL, 0);
        write16(GPT2_CTRL, 0);
        let _ = read16(GPT_STS);
        write32(IRQ_EOIOH0, u32::MAX);
        write32(IRQ_EOIOH1, u32::MAX);
        write32(IRQ_SENS_SET0, 1 << EKD_IRQ_GPT_LINE);
        write16(GPT1_PRESCALER, GPT_PRESCALER_div2 as u16);
        write16(GPT1_LEN, 128);
        write16(GPT1_CTRL, (GPT_CTRL_AUTOMODE | GPT_CTRL_Enable) as u16);
        write32(IRQ_MASK_CLR0, 1 << EKD_IRQ_GPT_LINE);
        DebugUart::log("[EMBASSY] GPT1 128Hz; IRQ23\n");
    }
    pub fn now() -> u32 {
        TIME.now() as u32
    }
    /// Called only by the thread-mode executor, never by an IRQ or task poll.
    pub fn poll_deadlines() {
        let now = TIME.now();
        critical_section::with(|cs| {
            TIME.timers.borrow(cs).borrow_mut().next_expiration(now);
        });
    }
    pub fn acknowledge() {
        let status = read16(GPT_STS);
        write32(IRQ_EOIOH0, 1 << EKD_IRQ_GPT_LINE);
        let _ = read32(IRQ_MASK0);
        if status as u32 & GPT_STS_1 != 0 {
            critical_section::with(|cs| {
                let ticks = TIME.ticks.borrow(cs);
                ticks.set(ticks.get().wrapping_add(1));
            });
        }
    }
    pub fn enable_irq() {
        unsafe {
            core::arch::asm!("mrs r0,cpsr", "bic r0,r0,#0x80", "msr cpsr_c,r0", out("r0") _, options(nostack));
        }
    }
    pub async fn delay_ms(ms: u64) {
        embassy_time::Timer::after_ticks((ms * 128 + 999) / 1000 + 1).await;
    }
}
