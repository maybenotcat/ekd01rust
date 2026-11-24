//! PSRAM bring-up traced through original BL 70006F10, 70006ED0 and 70006E3A.
use crate::{
    soc::{DebugUart, feed_watchdog, mmio::*, registers::*},
    stop,
};

const BOOT_CON: u32 = 0xa051_0000;
const EMI_RDCT: u32 = EMI_base + 0x88;
const EMI_DLLV: u32 = EMI_base + 0x90;
const EMI_IDLC: u32 = EMI_base + 0xd0;
const EMI_IDLD: u32 = EMI_base + 0xd8;
const EMI_IDLE: u32 = EMI_base + 0xe0;
const EMI_ODLC: u32 = EMI_base + 0xf8;
const EMI_ODLD: u32 = EMI_base + 0x100;
const EMI_ODLE: u32 = EMI_base + 0x108;
const EMI_ODLF: u32 = EMI_base + 0x110;
const EMI_IOA: u32 = EMI_base + 0x130;
const EMI_IOB: u32 = EMI_base + 0x138;
const EMI_DSRAM: u32 = EMI_base + 0x150;
const EMI_MSRAM: u32 = EMI_base + 0x158;
const EMI_ARBA: u32 = EMI_base + 0x170;
const EMI_ARBB: u32 = EMI_base + 0x178;
const EMI_ARBC: u32 = EMI_base + 0x180;
const EMI_MBISTA: u32 = EMI_base + 0x300;
const EMI_MBISTB: u32 = EMI_base + 0x308;
const EMI_MBISTD: u32 = EMI_base + 0x318;

fn settle(iterations: u32) {
    for _ in 0..iterations {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}
fn wait(register: u32, mask: u32, set: bool) {
    for _ in 0..1_000_000 {
        if (read32(register) & mask != 0) == set {
            return;
        }
        feed_watchdog();
    }
    DebugUart::hex("[BOOT] EMI_TIMEOUT_REGISTER", register);
    stop("[BOOT] EMI_TIMEOUT\n")
}
fn bist() -> bool {
    write32(EMI_MBISTA, 0);
    for direction in 0..2 {
        for pattern in [0x5aa5, 0xffff] {
            for inversion in 0..2 {
                write32(EMI_MBISTA, 0);
                write32(EMI_MBISTB, 0x1000_003f);
                write32(
                    EMI_MBISTA,
                    (pattern << 16) | (direction << 12) | (inversion << 13),
                );
                update32(EMI_MBISTA, 0, 0x325);
                wait(EMI_MBISTD, 2, true);
                if read32(EMI_MBISTD) & 1 != 0 {
                    write32(EMI_MBISTA, 0);
                    return false;
                }
            }
        }
    }
    write32(EMI_MBISTA, 0);
    true
}

pub fn initialize() {
    DebugUart::log("[BOOT] EMI_INIT_ENTER\n");
    settle(255);
    write16(BOOT_CON, 2);
    settle(255);
    update16(BOOT_CON, 0, 1);
    settle(127);
    // GFH EMI_LIST v5 contains thirteen N22 timing values after EMI_INF.
    // Read from the SRAM header loaded by BootROM, without touching NOR.
    let timing = |index: u32| read32(0x7000_60dc + index * 4);
    write32(EMI_IOA, timing(9));
    write32(EMI_IOB, timing(10));
    settle(127);
    write32(EMI_GENA, timing(0));
    write32(EMI_RDCT, timing(1));
    write32(EMI_DSRAM, timing(11));
    write32(EMI_MSRAM, timing(12));
    for (index, register) in [
        EMI_IDLC, EMI_IDLD, EMI_IDLE, EMI_ODLC, EMI_ODLD, EMI_ODLE, EMI_ODLF,
    ]
    .into_iter()
    .enumerate()
    {
        write32(register, timing(index as u32 + 2));
    }
    settle(127);
    let mut calibrated = false;
    for tap in (0..32u32).rev() {
        let delay = tap * 0x0101_0101;
        write32(EMI_IDLD, delay);
        write32(EMI_IDLC, delay);
        if bist() {
            calibrated = true;
            break;
        }
    }
    if !calibrated {
        for tap in 1..32u32 {
            write32(EMI_IDLE, tap << 16);
            if bist() {
                calibrated = true;
                break;
            }
        }
    }
    if !calibrated {
        stop("[BOOT] EMI_CALIBRATION_FAILED\n");
    }
    write32(EMI_DLLV, 0x300f_0000);
    wait(EMI_DLLV, 0x80, true);
    write32(EMI_DLLV, 0x700f_0000);
    wait(EMI_DLLV, 0x80, false);
    write32(EMI_DLLV, 0x100f_0000);
    write32(EMI_ARBA, 0x5431);
    write32(EMI_ARBB, 0x5009);
    write32(EMI_ARBC, 0x5046);
    DebugUart::log("[BOOT] EMI_INIT_EXIT\n");
}
