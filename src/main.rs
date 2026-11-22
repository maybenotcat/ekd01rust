#![no_std]
#![no_main]

extern crate alloc;

mod application;
mod bluetooth;
mod board;
mod crc;
mod input;
mod motor;
mod preferences;
mod protocol;
mod soc;
mod storage;
mod telemetry;
mod ui;

use core::panic::PanicInfo;

#[global_allocator]
static HEAP: embedded_alloc::LlffHeap = embedded_alloc::LlffHeap::empty();
include!(concat!(env!("OUT_DIR"), "/build_id.rs"));

core::arch::global_asm!(include_str!("../platform/entry.S"), options(raw));

#[unsafe(no_mangle)]
pub extern "C" fn firmware_main() -> ! {
    soc::DebugUart::log("[RUST] ENTRY; UART1 115200 inherited\n");
    soc::DebugUart::log("[RUST] BUILD_ID=");
    soc::DebugUart::log(BUILD_ID);
    soc::DebugUart::log("\n");
    soc::Watchdog::feed();
    if !soc::memory::Memory::probe_heap() {
        panic!("PSRAM heap probe failed");
    }
    let (start, size) = soc::memory::Memory::heap_range();
    unsafe {
        HEAP.init(start, size);
    }
    soc::memory::Memory::initialize_cache();
    soc::time::Timer::initialize_interrupts();
    soc::time::Timer::enable_irq();
    static EXECUTOR: static_cell::StaticCell<embassy_executor::raw::Executor> =
        static_cell::StaticCell::new();
    let executor = EXECUTOR.init(embassy_executor::raw::Executor::new(core::ptr::null_mut()));
    let spawner = executor.spawner();
    spawner.spawn(watchdog_task().unwrap());
    spawner.spawn(application_task(spawner).unwrap());
    loop {
        soc::time::Timer::poll_deadlines();
        // This is the sole poll site, in thread mode. IRQs only enqueue work;
        // no task or interrupt may recursively poll this executor.
        unsafe { executor.poll() };
    }
}

#[embassy_executor::task]
async fn watchdog_task() {
    loop {
        soc::Watchdog::feed();
        embassy_time::Timer::after_ticks(16).await;
    }
}

#[embassy_executor::task]
async fn application_task(spawner: embassy_executor::Spawner) {
    application::Application::initialize(spawner)
        .await
        .run()
        .await;
}

#[unsafe(no_mangle)]
pub extern "C" fn fault_report(kind: u32, lr: u32, status: u32) -> ! {
    soc::DebugUart::hex("[FAULT] KIND", kind);
    soc::DebugUart::hex("[FAULT] LR", lr);
    soc::DebugUart::hex("[FAULT] SPSR", status);
    loop {
        core::hint::spin_loop();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn irq_dispatch() {
    use soc::{mmio::read32, registers::*};
    let status = read32(IRQ_STA2);
    if status & IRQ_STA2_INVALID != 0 {
        return;
    }
    match status & IRQ_STA2_LINE_MASK {
        EKD_IRQ_GPT_LINE => soc::time::Timer::acknowledge(),
        EKD_IRQ_UART3_LINE => soc::bridge_uart::BridgeUart::irq(),
        EKD_IRQ_BTIF_LINE => soc::btif::Btif::irq(),
        other => fault_report(0x100 | other, 0, 0),
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    use core::fmt::Write;
    let _ = writeln!(soc::DebugUart, "[PANIC] {info}");
    loop {
        core::hint::spin_loop();
    }
}
