#![doc = " Clock-controller primitives for the 26 MHz MT2502 platform."]
use crate::{mmio::*, registers::*, watchdog::Watchdog};
#[derive(Clone, Copy)]
pub struct Configuration {
    pub mpll_control: u16,
    pub mpll_auxiliary: u16,
    pub domain_h: u32,
    pub domain_c: u32,
}
pub struct Clocks;
impl Clocks {
    pub fn delay_us(microseconds: u32) -> Result<(), crate::error::Error> {
        const GPT_FREERUN_CONTROL: u32 = 0xa00c_0028;
        const GPT_FREERUN_COUNT: u32 = 0xa00c_002c;
        write32(GPT_FREERUN_CONTROL, 0);
        update32(GPT_FREERUN_CONTROL, 0, 1);
        let start = read32(GPT_FREERUN_COUNT);
        let ticks = 26 * microseconds + (42 * microseconds).div_ceil(1000);
        for _ in 0..1_000_000 {
            if read32(GPT_FREERUN_COUNT).wrapping_sub(start) >= ticks {
                return Ok(());
            }
            Watchdog::feed();
        }
        return Err(crate::error::Error::GptTimeout);
    }
    pub fn stabilize_32k() -> Result<(), crate::error::Error> {
        const MD_HW_TDMA_START: u32 = 0x8300_0018;
        const USCNT_CON: u32 = 0xa018_0080;
        const USCNT_START_CODE: u32 = 0x1153_0001;
        const F32K_CHECK_CTRL: u32 = 0xa018_00a0;
        const F32K_CHECK_WINDOW: u32 = 0xa018_00a4;
        const F32K_CHECK_START: u32 = 0xa018_00a8;
        const F32K_CHECK_END: u32 = 0xa018_00ac;
        write32(MD_HW_TDMA_START, 1);
        for _ in 0..1000 {
            unsafe {
                core::arch::asm!("nop", options(nomem, nostack));
            }
        }
        write32(USCNT_CON, USCNT_START_CODE);
        write32(F32K_CHECK_WINDOW, 2);
        for _ in 0..80_000 {
            write32(F32K_CHECK_CTRL, 0x11);
            let mut complete = false;
            for _ in 0..100_000 {
                if read32(F32K_CHECK_CTRL) & 1 == 0 {
                    complete = true;
                    break;
                }
            }
            if !complete {
                return Err(crate::error::Error::Clock32kBusy);
            }
            let begin = read32(F32K_CHECK_START) & 0xff_ffff;
            let begin_fraction = read32(F32K_CHECK_START) >> 24;
            let mut end = read32(F32K_CHECK_END) & 0xff_ffff;
            let end_fraction = read32(F32K_CHECK_END) >> 24;
            if end <= begin {
                end += 0x100_0000;
            }
            let interval = (26 * end + end_fraction).wrapping_sub(26 * begin + begin_fraction);
            if interval.wrapping_sub(0x515) < 0x257 {
                write32(F32K_CHECK_CTRL, 0x110);

                return Ok(());
            }
            Watchdog::feed();
        }
        return Err(crate::error::Error::Clock32kUnstable);
    }
    pub fn initialize_clocks(
        usb_boot: bool,
        configuration: Configuration,
    ) -> Result<(), crate::error::Error> {
        Self::initialize_bus_pads();
        Self::initialize_pll(usb_boot, configuration)?;
        Ok(())
    }
    pub fn initialize_bus_pads() {
        const MCU_BUS_DCM_CONTROL: u32 = 0xa001_0600;
        for offset in [0, 4, 8] {
            update32(MCU_BUS_DCM_CONTROL + offset, 0x0030_0030, 0);
        }
        const EFUSE_IO_PACKAGE: u32 = 0xa011_0000;
        if read32(EFUSE_IO_PACKAGE) & 7 == 2 {
            update32(0xa002_0b10, 0x0007_e000, 0);
            update32(0xa002_0b30, 0x0007_e000, 0);
            write32(
                0xa002_0810,
                (read32(0xa002_0810) & 0x0030_0000).wrapping_add(0xff8f_ffff),
            );
        }
    }
    fn initialize_pll(
        usb_boot: bool,
        configuration: Configuration,
    ) -> Result<(), crate::error::Error> {
        if usb_boot {
            write32(CLK_CONDC, 0x8008);
            let _ = read32(CLK_CONDC);
            let _ = read32(CLK_CONDC);
            if read32(CLK_CONDC) & 2 == 0 {
                Self::delay_us(1)?;
            }
            write32(CLK_CONDC, 8);
        }
        const PLL_RESERVED0: u32 = 0xa017_0048;
        const PLL_RESERVED1: u32 = 0xa017_004c;
        const PLL_RESERVED2: u32 = 0xa017_0700;
        const PLL_REFERENCE: u32 = 0xa017_0140;
        const PLL_BIAS: u32 = 0xa017_0640;
        const PLL_CLOCK_GATE: u32 = 0xa017_0050;
        write16(PLL_RESERVED0, 0);
        write16(PLL_RESERVED1, 0);
        write16(PLL_RESERVED2, 0);
        Self::delay_us(1)?;
        update16(PLL_REFERENCE, 0, 1);
        write16(MPLL_CON0, configuration.mpll_control & !1);
        write16(MPLL_CON1, configuration.mpll_auxiliary);
        write16(PLL_BIAS, 1);
        update16(MPLL_CON0, 0, 1);
        Self::delay_us(50)?;
        update16(PLL_CLOCK_GATE, 4, 0);
        update32(CLK_CONDA, 0, 0x100);
        write32(CLK_CONDH, configuration.domain_h);
        write32(CLK_CONDC, 0x8008);
        Self::delay_us(2)?;
        write32(CLK_CONDC, configuration.domain_c);
        Ok(())
    }
}

pub fn domain_c_control() -> u32 {
    read32(CLK_CONDC)
}
