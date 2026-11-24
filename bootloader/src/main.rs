#![no_std]
#![no_main]

mod emi;
mod rtc;
mod sfi;
mod sfi_profile;
mod soc;
mod storage;

use ekd01_boot_contract as layout;
use embassy_boot::{AlignedBuffer, BootLoader, BootLoaderConfig};
use soc::DebugUart;

core::arch::global_asm!(
    ".equ BOOT_HANDOFF_MAGIC, {magic}",
    ".equ BOOT_HANDOFF_VERSION, {version}",
    include_str!("../entry.S"),
    magic = const layout::HANDOFF_MAGIC,
    version = const layout::HANDOFF_VERSION,
);

pub fn stop(message: &str) -> ! {
    DebugUart::log(message);
    // Keep the error readable. Recovery remains the physical BootROM USB path.
    loop {
        soc::feed_watchdog();
    }
}

#[unsafe(no_mangle)]
extern "C" fn boot_fault(kind: u32, lr: u32, spsr: u32) -> ! {
    DebugUart::hex("[BOOT] EXCEPTION", kind);
    DebugUart::hex("[BOOT] EXCEPTION_LR", lr);
    DebugUart::hex("[BOOT] EXCEPTION_SPSR", spsr);
    stop("[BOOT] HALTED\n")
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    stop("[BOOT] PANIC\n")
}

#[unsafe(no_mangle)]
extern "C" fn boot_main(parameters: u32) -> ! {
    // BootROM passes its v4 parameter block in R4, as used by original BL.
    // Do not dereference arbitrary data on recovery paths without that block.
    let usb_boot = parameters & 3 == 0
        && (0x7000_0000..0x7000_5758).contains(&parameters)
        && soc::mmio::read32(parameters + 2216) & 2 != 0;
    soc::initialize_clocks(usb_boot);
    soc::initialize_uart();
    DebugUart::log("[BOOT] EKD01 EMBASSY; single loader; UART1 115200\n");
    emi::initialize();
    rtc::initialize_oscillator();
    soc::stabilize_32k();
    DebugUart::hex(
        "[BOOT] EMI_GENA",
        soc::mmio::read32(soc::registers::EMI_GENA),
    );
    if !soc::probe_psram() {
        stop("[BOOT] PSRAM_PROBE_FAILED\n");
    }
    DebugUart::log("[BOOT] PSRAM_OK\n");
    soc::prepare_runtime();
    sfi::initialize();
    soc::initialize_power();
    soc::release_usb_download_power();
    soc::latch_power();
    let active = storage::PartitionFlash::new(layout::ACTIVE);
    let dfu = storage::PartitionFlash::new(layout::DFU);
    let state = storage::PartitionFlash::new(layout::STATE);
    let mut loader = BootLoader::new(BootLoaderConfig { active, dfu, state });
    let mut buffer = AlignedBuffer([0u8; 256]);
    let result = loader
        .prepare_boot(&mut buffer.0)
        .and_then(|_| loader.read_state(&mut buffer.0));
    let trial = matches!(result, Ok(embassy_boot::State::Swap));
    match result {
        Ok(embassy_boot::State::Swap) => DebugUart::log("[BOOT] SWAPPED; trial application\n"),
        Ok(embassy_boot::State::Revert) => DebugUart::log("[BOOT] REVERTED\n"),
        Ok(_) => DebugUart::log("[BOOT] ACTIVE\n"),
        Err(_) => stop("[BOOT] PREPARE_FAILED\n"),
    }
    let header = unsafe {
        core::slice::from_raw_parts(
            (layout::NOR_BASE + layout::ACTIVE.start) as *const u8,
            layout::APPLICATION_HEADER_BYTES as usize + 64,
        )
    };
    if layout::validate_application(header).is_err() {
        stop("[BOOT] INVALID_APPLICATION\n");
    }
    DebugUart::hex("[BOOT] JUMP", layout::APPLICATION_ENTRY);
    soc::finish_handoff();
    soc::arm_application_watchdog();
    unsafe extern "C" {
        fn boot_jump(entry: u32, trial: u32) -> !;
    }
    unsafe { boot_jump(layout::APPLICATION_ENTRY, trial as u32) }
}
