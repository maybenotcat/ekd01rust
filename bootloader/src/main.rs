#![no_std]
#![no_main]

type Emi = mt2502_hal::boot::emi::Emi<soc::Environment>;
type RtcBoot = mt2502_hal::boot::rtc::RtcBoot<soc::Environment>;
type Sfi = mt2502_hal::boot::sfi::Sfi<soc::Environment>;

mod soc;
mod storage;

use ekd01_boot_contract as layout;
use embassy_boot::{AlignedBuffer, BootLoader, BootLoaderConfig};
use soc::DebugUart;
include!(concat!(env!("OUT_DIR"), "/build_id.rs"));

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
        mt2502_hal::watchdog::Watchdog::feed();
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
    soc::Hardware::initialize_clocks(usb_boot);
    soc::Hardware::initialize_uart();
    DebugUart::log("[BOOT] EKD01 EMBASSY; single loader; UART1 115200\n");
    DebugUart::log("[BOOT] BUILD_ID=");
    DebugUart::log(BUILD_ID);
    DebugUart::log("\n");
    Emi::initialize();
    RtcBoot::initialize_oscillator();
    soc::Hardware::stabilize_32k();
    DebugUart::hex(
        "[BOOT] EMI_GENA",
        soc::mmio::read32(soc::registers::EMI_GENA),
    );
    if !soc::Hardware::probe_psram([0x00100000, 0x00300000, 0x006efffc]) {
        stop("[BOOT] PSRAM_PROBE_FAILED\n");
    }
    DebugUart::log("[BOOT] PSRAM_OK\n");
    soc::Hardware::prepare_runtime(0x70006140);
    Sfi::initialize();
    soc::Hardware::initialize_power();
    soc::Hardware::release_usb_download_power();
    soc::Hardware::latch_power();
    // Exclusive physical owner; all three Embassy Boot views borrow this device.
    let controller = unsafe { soc::flash::FlashController::attach() };
    DebugUart::hex("[FLASH] JEDEC", controller.jedec().unwrap_or(0));
    let active = storage::partition(&controller, layout::ACTIVE);
    let dfu = storage::partition(&controller, layout::DFU);
    let state = storage::partition(&controller, layout::STATE);
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
    soc::Hardware::finish_handoff();
    soc::Hardware::arm_application_watchdog();
    unsafe extern "C" {
        fn boot_jump(entry: u32, trial: u32) -> !;
    }
    unsafe { boot_jump(layout::APPLICATION_ENTRY, trial as u32) }
}
