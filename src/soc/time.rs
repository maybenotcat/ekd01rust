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
    /// Prove the IRQ-driven timebase before awaiting it. GPT4 is the independent
    /// 26MHz counter used by the original loader for PLL settling, not Embassy time.
    pub fn verify_progress() {
        update32(GPT4_CTRL, 0, GPT4_ENABLE);
        let start = read32(GPT4_COUNT);
        let tick = Self::now();
        // The iteration cap also bounds failure if GPT4 itself is stopped.
        for _ in 0..5_000_000 {
            if Self::now().wrapping_sub(tick) >= 2 {
                DebugUart::log("[EMBASSY] TICK_IRQ_OK\n");
                return;
            }
            if read32(GPT4_COUNT).wrapping_sub(start) >= 100 * EKD_GPT4_TICKS_PER_MS {
                break;
            }
        }
        let cpsr: u32;
        unsafe { core::arch::asm!("mrs {0}, cpsr", out(reg) cpsr, options(nostack)); }
        DebugUart::hex("[EMBASSY] CPSR", cpsr);
        DebugUart::hex("[EMBASSY] TICKS", Self::now());
        DebugUart::hex("[EMBASSY] GPT4_ELAPSED", read32(GPT4_COUNT).wrapping_sub(start));
        DebugUart::hex("[EMBASSY] GPT1_CTRL", read16(GPT1_CTRL) as u32);
        DebugUart::hex("[EMBASSY] GPT1_LEN", read16(GPT1_LEN) as u32);
        DebugUart::hex("[EMBASSY] GPT1_PRESCALER", read16(GPT1_PRESCALER) as u32);
        DebugUart::hex("[EMBASSY] IRQ_MASK0", read32(IRQ_MASK0));
        DebugUart::hex("[EMBASSY] IRQ_STA2", read32(IRQ_STA2));
        // Reading GPT_STS acknowledges its pending bits; only do it on failure.
        DebugUart::hex("[EMBASSY] GPT_STS", read16(GPT_STS) as u32);
        DebugUart::hex("[EMBASSY] RTC_XOSCCAL", read16(RTC_XOSCCAL) as u32);
        // Address zero is a physical exception vector, so use an instruction
        // rather than constructing a null Rust pointer for volatile access.
        for address in [0x18u32, 0x38] {
            let value: u32;
            unsafe { core::arch::asm!("ldr {value}, [{address}]", value = out(reg) value,
                address = in(reg) address, options(nostack, readonly)); }
            DebugUart::hex("[EMBASSY] IRQ_VECTOR", value);
        }
        panic!("GPT1 IRQ timebase stopped");
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
