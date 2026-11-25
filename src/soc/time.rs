use super::DebugUart;
use core::{
    cell::{Cell, RefCell},
    task::Waker,
};
use critical_section::Mutex;
use embassy_time_driver::Driver;
use embassy_time_queue_utils::queue_generic::ConstGenericQueue;
use mt2502_hal::gpt::Gpt1;

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
        Gpt1::initialize_128hz();
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
        if Gpt1::acknowledge() {
            critical_section::with(|cs| {
                let ticks = TIME.ticks.borrow(cs);
                ticks.set(ticks.get().wrapping_add(1));
            });
        }
    }
    pub fn enable_irq() {
        mt2502_hal::cpu::enable_irq();
    }
}
