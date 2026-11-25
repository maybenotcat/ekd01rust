pub trait TimingSource {
    fn word(&self, index: u32) -> u32;
}
pub struct Emi;
use crate::{mmio::*, registers::*, watchdog::Watchdog};
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
impl Emi {
    fn settle(iterations: u32) {
        for _ in 0..iterations {
            unsafe {
                core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
            }
        }
    }
    fn wait(register: u32, mask: u32, set: bool) -> Result<(), crate::error::Error> {
        for _ in 0..1_000_000 {
            if (read32(register) & mask != 0) == set {
                return Ok(());
            }
            Watchdog::feed();
        }

        return Err(crate::error::Error::EmiTimeout { register });
    }
    fn bist() -> Result<bool, crate::error::Error> {
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
                    Self::wait(EMI_MBISTD, 2, true)?;
                    if read32(EMI_MBISTD) & 1 != 0 {
                        write32(EMI_MBISTA, 0);
                        return Ok(false);
                    }
                }
            }
        }
        write32(EMI_MBISTA, 0);
        Ok(true)
    }
    pub fn initialize(
        source: &impl TimingSource,
        arbitration: [u32; 3],
    ) -> Result<(), crate::error::Error> {
        Self::settle(255);
        write16(BOOT_CON, 2);
        Self::settle(255);
        update16(BOOT_CON, 0, 1);
        Self::settle(127);
        let timing = |index: u32| source.word(index);
        write32(EMI_IOA, timing(9));
        write32(EMI_IOB, timing(10));
        Self::settle(127);
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
        Self::settle(127);
        let mut calibrated = false;
        for tap in (0..32u32).rev() {
            let delay = tap * 0x0101_0101;
            write32(EMI_IDLD, delay);
            write32(EMI_IDLC, delay);
            if Self::bist()? {
                calibrated = true;
                break;
            }
        }
        if !calibrated {
            for tap in 1..32u32 {
                write32(EMI_IDLE, tap << 16);
                if Self::bist()? {
                    calibrated = true;
                    break;
                }
            }
        }
        if !calibrated {
            return Err(crate::error::Error::EmiCalibrationFailed);
        }
        write32(EMI_DLLV, 0x300f_0000);
        Self::wait(EMI_DLLV, 0x80, true)?;
        write32(EMI_DLLV, 0x700f_0000);
        Self::wait(EMI_DLLV, 0x80, false)?;
        write32(EMI_DLLV, 0x100f_0000);
        write32(EMI_ARBA, arbitration[0]);
        write32(EMI_ARBB, arbitration[1]);
        write32(EMI_ARBC, arbitration[2]);

        Ok(())
    }
}

pub fn configuration_word() -> u32 {
    read32(EMI_GENA)
}
