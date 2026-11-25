use crate::environment::BootEnvironment;
pub struct Startup<P: BootEnvironment>(core::marker::PhantomData<P>);
use super::rtc::RtcBoot;
use crate::{mmio::*, pmu::data as pmu_data, registers::*, watchdog::Watchdog};
impl<P: BootEnvironment> Startup<P> {
    pub fn delay_us(microseconds: u32) {
        const GPT_FREERUN_CONTROL: u32 = 0xa00c_0028;
        const GPT_FREERUN_COUNT: u32 = 0xa00c_002c;
        write32(GPT_FREERUN_CONTROL, 0);
        update32(GPT_FREERUN_CONTROL, 0, 1);
        let start = read32(GPT_FREERUN_COUNT);
        let ticks = 26 * microseconds + (42 * microseconds).div_ceil(1000);
        for _ in 0..1_000_000 {
            if read32(GPT_FREERUN_COUNT).wrapping_sub(start) >= ticks {
                return;
            }
            Watchdog::feed();
        }
        P::stop("[BOOT] GPT_TIMEOUT\n")
    }

    /// Original BL WaitFor32KStable, 700069c0/70006ba2/70006bac.
    pub fn stabilize_32k() {
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
                P::stop("[BOOT] 32K_BUSY\n");
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
                P::log("[BOOT] 32K_STABLE\n");
                return;
            }
            Watchdog::feed();
        }
        P::stop("[BOOT] 32K_UNSTABLE\n");
    }

    /// EXT_BL 10028358/10028ca4: PMU startup precedes the RTC power latch.
    pub fn initialize_power() {
        // EXT_BL 10027fb0 and 10027fd0 run before DclPMU_Initialize.
        update16(CHR_CON0, RG_VCDT_HV_VTH_MASK, 11 << RG_VCDT_HV_VTH_SHIFT);
        update16(CHR_CON0, RG_VCDT_HV_EN_MASK, RG_VCDT_HV_EN_MASK);
        let usb_without_battery = read16(EKD_BOOT_FLAGS) as u32 & EKD_BOOT_USB_WITHOUT_BATTERY != 0;
        if !usb_without_battery && read16(CHR_CON0 + 4) & 8 != 0 && read16(CHR_CON0) & 0x20 == 0 {
            // Original current lookup floors 50000 to 45000 (table index 12).
            update16(CHR_CON0, 0xf000, 12 << 12);
            update16(CHR_CON0 + 8, 15, 12);
            update16(CHR_CON0, 1, 1);
            update16(CHR_CON0, 8, 8);
        }
        if !usb_without_battery {
            for &(offset, mask, bits) in pmu_data::STARTUP {
                update16(PMU_base + offset, mask, bits);
            }
        }
        // DclPMU_Initialize enables ADIE interrupt delivery, then applies board settings.
        update16(0xa073_0010, 0, 0x10);
        // N22 custom PMU callback 10028e10, not the SDK's unrelated board callback.
        update16(KPLED_CON0, KPLED_MODE_MASK, 0);
        update16(SPK_CON0, RG_SPK_VOL_MASK, 3 << RG_SPK_VOL_SHIFT); // +12dB
        update16(ISINKS_CON0, ISINKS_MODE_MASK, 0);
        for register in [ISINK0_CON0, ISINK1_CON0] {
            update16(register, ISINK_STEP_MASK, 5 << ISINK_STEP_SHIFT); // 24mA
        }
        update16(TEST_CON0, RG_PWRKEY_RST_EN_MASK, RG_PWRKEY_RST_EN_MASK);
        update16(
            TEST_CON0,
            RG_LNGP_SHUTDOWN_SEL_MASK,
            RG_LNGP_SHUTDOWN_SEL_MASK,
        );
        for bits in [0, RG_PWRKEY_RST_FUNC_SET_MASK, 0] {
            update16(TEST_CON0, RG_PWRKEY_RST_FUNC_SET_MASK, bits);
        }
        P::log("[BOOT] PMU_READY\n");
    }

    /// Original EXT_BL 10027b7c disables cache mappings before JumpCmd.
    pub fn finish_handoff() {
        update16(CACHE_CON, 3, 0);
        write32(CACHE_REGION_EN, 0);
        while read16(UART_LSR(UART1_base)) as u32 & 0x40 == 0 {
            Watchdog::feed();
        }
        // SRAM handoff 7000505a repeats this immediately before JumpCmd.
        write16(0xa051_0000, 3);
    }

    /// Register order from original BL sub_70006684 and its preceding pad setup.
    pub fn initialize_clocks(usb_boot: bool) {
        // SetPLLEMI calls custom_setEMI (70006e0c) before pad/PLL setup.
        write16(0xa051_0000, 2);
        Self::initialize_bus_pads();
        Self::initialize_pll(usb_boot);
    }

    /// Both BL 700065e4 and EXT_BL 70005098 execute these writes.
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

    fn initialize_pll(usb_boot: bool) {
        if usb_boot {
            write32(CLK_CONDC, 0x8008);
            let _ = read32(CLK_CONDC);
            let _ = read32(CLK_CONDC);
            if read32(CLK_CONDC) & 2 == 0 {
                Self::delay_us(1);
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
        Self::delay_us(1);
        update16(PLL_REFERENCE, 0, 1);
        write16(MPLL_CON0, 0x1400);
        write16(MPLL_CON1, 0);
        write16(PLL_BIAS, 1);
        update16(MPLL_CON0, 0, 1);
        Self::delay_us(50);
        update16(PLL_CLOCK_GATE, 4, 0);
        update32(CLK_CONDA, 0, 0x100);
        write32(CLK_CONDH, 0xd001);
        write32(CLK_CONDC, 0x8008);
        Self::delay_us(2);
        write32(CLK_CONDC, 0xb620);
    }

    pub fn initialize_uart() {
        // Original BL divider 28, 52MHz UART clock, 115200 8N1.
        write16(UART1_base + 0x0c, 0x83);
        write16(UART1_base, 28);
        write16(UART1_base + 4, 0);
        write16(UART1_base + 0x0c, 3);
        write16(UART1_base + 8, 0x47);
        write16(UART1_base + 0x10, 3);
        write16(UART1_base + 4, 0);
        write16(UART1_base + 0x24, 0);
    }

    /// RTC_PWIC_PowerOn_RTC_INIT on the MT6261-family RTC: set BBPU power
    /// and auto power-down control, then retain the original 2ms key debounce.
    pub fn latch_power() {
        RtcBoot::<P>::restore_running_clock();
        RtcBoot::<P>::initialize_power_on_state();
        const RTC_POWERON: u32 = 0x430c;
        update16(RTC_BBPU, 0, RTC_POWERON);
        RtcBoot::<P>::commit();
        if read16(RTC_BBPU) as u32 & EKD_RTC_BBPU_POWER_LATCH == 0 {
            P::stop("[BOOT] POWER_LATCH_FAILED\n");
        }
        const RTC_INFO1: u32 = RTC_base + 0x58;
        update16(RTC_INFO1, 0x0e, 0);
        // EXT_BL 1002f398 copies weekday while preserving the alarm spare byte.
        write16(
            RTC_base + 0x40,
            (read16(RTC_base + 0x40) & 0xff00) | read16(RTC_base + 0x24),
        );
        RtcBoot::<P>::commit();
        P::log("[BOOT] POWER_LATCHED\n");
    }

    /// EXT_BL 10027450 dispatches charging command 8 to 10029c10.
    /// This is RG_USBDL_RST, not the charger watchdog (command 12).
    /// Preserve the original halfword read/modify/write before the RTC latch.
    pub fn release_usb_download_power() {
        const RG_USBDL_RST_MASK: u32 = 0x4000;
        // 1002712a..10027140 repeats this dispatch twice, each guarded by
        // the USB-without-battery boot flag (10034bb4).
        for _ in 0..2 {
            if read16(EKD_BOOT_FLAGS) as u32 & EKD_BOOT_USB_WITHOUT_BATTERY == 0 {
                update16(CHR_CON10, RG_USBDL_RST_MASK, RG_USBDL_RST_MASK);
            }
        }
        P::log("[BOOT] USBDL_POWER_PATH_READY\n");
    }

    pub fn probe_psram(locations: [u32; 3]) -> bool {
        // Preserve every location: the application has not initialized its RAM yet.
        let saved = locations.map(read32);
        for (index, address) in locations.iter().enumerate() {
            write32(*address, 0xa55a_1234 ^ index as u32);
        }
        let valid = locations
            .iter()
            .enumerate()
            .all(|(index, address)| read32(*address) == 0xa55a_1234 ^ index as u32);
        for (address, value) in locations.into_iter().zip(saved) {
            write32(address, value);
        }
        valid
    }

    pub fn prepare_runtime(vector_source: u32) {
        // Every instruction executes from internal SRAM. Invalidate inherited XIP
        // cache before modifying NOR; leave the application to configure its cache.
        update16(CACHE_CON, EKD_CACHE_DISABLE_MASK, 0);
        write32(CACHE_OP, EKD_CACHE_INVALIDATE_ALL);
        write32(CACHE_REGION_EN, 0);
        for offset in (0..64).step_by(4) {
            let word = read32(vector_source + offset);
            // Physical vector zero is valid hardware memory, not a Rust pointer.
            unsafe {
                core::arch::asm!("str {word}, [{address}]", word = in(reg) word,
            address = in(reg) offset, options(nostack, preserves_flags));
            }
        }
    }

    pub fn arm_application_watchdog() {
        const WDT_MODE: u32 = RGU_base;
        const WDT_LENGTH: u32 = RGU_base + 4;
        const WDT_LENGTH_MAX_WITH_KEY: u16 = 0xffe8;
        const WDT_MODE_KEY: u16 = 0x2200;
        const WDT_MODE_ENABLE: u16 = 1;
        // Internal watchdog reset, no external reset output and no IRQ-only mode.
        write16(WDT_LENGTH, WDT_LENGTH_MAX_WITH_KEY);
        write16(WDT_MODE, WDT_MODE_KEY | WDT_MODE_ENABLE);
        Watchdog::feed();
    }
}
