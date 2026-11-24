// Hardware constants from the verified C reference. No driver implementation is imported.
#![allow(non_snake_case, non_upper_case_globals, dead_code, unused_parens)]
pub const CONFIG_base: u32 = 0xa0010000;
pub const GPIO_base: u32 = 0xa0020000;
pub const RGU_base: u32 = 0xa0030000;
pub const EMI_base: u32 = 0xa0050000;
pub const CIRQ_base: u32 = 0xa0060000;
pub const UART1_base: u32 = 0xa0080000;
pub const UART3_base: u32 = 0xa00a0000;
pub const BTIF_base: u32 = 0xa00b0000;
pub const GPT_base: u32 = 0xa00c0000;
pub const SFI_base: u32 = 0xa0140000;
pub const MIXED_base: u32 = 0xa0170000;
pub const EFUSE_base: u32 = 0xa01c0000;
pub const PWM2_base: u32 = 0xa0280000;
pub const LCD_base: u32 = 0xa0450000;
pub const L1_CACHE_base: u32 = 0xa0530000;
pub const MPU_base: u32 = 0xa0540000;
pub const PMU_base: u32 = 0xa0700000;
pub const RTC_base: u32 = 0xa0710000;
pub const ANA_CFGSYS_base: u32 = 0xa0730000;
pub const PWM_2CH_base: u32 = 0xa0740000;
pub const BTIF_RBR_ADDR: u32 = UART_RBR(BTIF_base);
pub const BTIF_THR_ADDR: u32 = UART_THR(BTIF_base);
pub const BTIF_IER_ADDR: u32 = UART_IER(BTIF_base);
pub const BTIF_IIR_ADDR: u32 = UART_IIR(BTIF_base);
pub const BTIF_LSR_ADDR: u32 = UART_LSR(BTIF_base);
pub const BTIF_SLEEP_EN_ADDR: u32 = UART_SLEEP_EN(BTIF_base);
pub const BTIF_DMA_EN_ADDR: u32 = UART_RXDMA(BTIF_base);
pub const BTIF_LSR_DR: u32 = UART_LSR_DR;
pub const BTIF_LSR_OE: u32 = UART_LSR_OE;
pub const BTIF_LSR_TEMT: u32 = UART_LSR_TEMT;
pub const BTIF_IER_RXEN: u32 = UART_IER_ERBFI;
pub const CACHE_CON: u32 = (L1_CACHE_base + 0x00);
pub const CACHE_OP: u32 = (L1_CACHE_base + 0x04);
pub const CACHE_HCNT0L: u32 = (L1_CACHE_base + 0x08);
pub const CACHE_CCNT1U: u32 = (L1_CACHE_base + 0x24);
pub const CACHE_REGION_EN: u32 = (L1_CACHE_base + 0x2c);
pub const fn MPU_PROT(channel: u32) -> u32 {
    (MPU_base + (channel) * 4)
}
pub const fn MPU_CACHE(channel: u32) -> u32 {
    (MPU_base + 0x40 + (channel) * 4)
}
pub const EKD_CACHE_DISABLE_MASK: u32 = 0x000f;
pub const EKD_CACHE_RECONFIGURE_MASK: u32 = 0x030f;
pub const EKD_CACHE_32K_4WAY_CONTROL: u32 = 0x838f;
pub const EKD_CACHE_INVALIDATE_ALL: u32 = 0x03;
pub const EKD_CACHE_XIP_START: u32 = 0x10050180;
pub const EKD_MEMORY_PAGE_MASK: u32 = 4095;
pub const IRQ_MASK0: u32 = (CIRQ_base + 0x000);
pub const IRQ_MASK1: u32 = (CIRQ_base + 0x004);
pub const IRQ_MASK_SET0: u32 = (CIRQ_base + 0x020);
pub const IRQ_MASK_SET1: u32 = (CIRQ_base + 0x024);
pub const IRQ_MASK_CLR0: u32 = (CIRQ_base + 0x040);
pub const IRQ_MASK_CLR1: u32 = (CIRQ_base + 0x044);
pub const IRQ_SENS_SET0: u32 = (CIRQ_base + 0x080);
pub const IRQ_SENS_SET1: u32 = (CIRQ_base + 0x084);
pub const IRQ_SOFT_CLR0: u32 = (CIRQ_base + 0x100);
pub const IRQ_SOFT_CLR1: u32 = (CIRQ_base + 0x104);
pub const IRQ_EOIOH0: u32 = (CIRQ_base + 0x160);
pub const IRQ_EOIOH1: u32 = (CIRQ_base + 0x164);
pub const IRQ_FSEL0: u32 = (CIRQ_base + 0x180);
pub const IRQ_FSEL1: u32 = (CIRQ_base + 0x184);
pub const IRQ_STA2: u32 = (CIRQ_base + 0x1a0);
pub const fn IRQ_SEL(index: u32) -> u32 {
    (CIRQ_base + 0x200 + 4 * (index))
}
pub const EKD_IRQ_GPT_LINE: u32 = 23;
pub const EKD_IRQ_LCD_LINE: u32 = 28;
pub const EKD_IRQ_UART3_LINE: u32 = 29;
pub const EKD_IRQ_BTIF_LINE: u32 = 43;
pub const IRQ_STA2_INVALID: u32 = 0x100;
pub const IRQ_STA2_LINE_MASK: u32 = 0xff;
pub const EKD_IRQ_ALL_MASK: u32 = 0xffffffff;
pub const EKD_IRQ_PATCH_DISABLED: u32 = 0x2f2f2f2f;
pub const GPIO_BANK_STRIDE: u32 = 0x10;
pub const GPIO_SET_ALIAS_OFFSET: u32 = 4;
pub const GPIO_CLR_ALIAS_OFFSET: u32 = 8;
pub const GPIO_BANK_PINS: u32 = 32;
pub const GPIO_MODE_ONE_CONTROL_BITS: u32 = 4;
pub const GPIO_MODE_ONE_REG_CONTROL_NUM: u32 = 8;
pub const GPIO_MODE_FIELD_MASK: u32 = 0x0f;
pub const GPIO_DIR0: u32 = (GPIO_base + 0x000);
pub const GPIO_DIR0_SET: u32 = (GPIO_base + 0x004);
pub const GPIO_DIR0_CLR: u32 = (GPIO_base + 0x008);
pub const GPIO_PULL0: u32 = (GPIO_base + 0x100);
pub const GPIO_PULL0_SET: u32 = (GPIO_base + 0x104);
pub const GPIO_PULL0_CLR: u32 = (GPIO_base + 0x108);
pub const GPIO_DINV0: u32 = (GPIO_base + 0x200);
pub const GPIO_OUTPUT_LEVEL0: u32 = (GPIO_base + 0x300);
pub const GPIO_OUTPUT_LEVEL0_SET: u32 = (GPIO_base + 0x304);
pub const GPIO_OUTPUT_LEVEL0_CLR: u32 = (GPIO_base + 0x308);
pub const GPIO_INPUT_LEVEL0: u32 = (GPIO_base + 0x400);
pub const GPIO_PULLSEL0_SET: u32 = (GPIO_base + 0x504);
pub const GPIO_PULLSEL0: u32 = (GPIO_base + 0x500);
pub const GPIO_DRV0: u32 = (GPIO_base + 0x800);
pub const EKD_GPIO_DRV_BANK1: u32 = (GPIO_DRV0 + GPIO_BANK_STRIDE);
pub const GPIO_PUPD0_CLR: u32 = (GPIO_base + 0xa08);
pub const GPIO_R0_CLR_REG0: u32 = (GPIO_base + 0xb08);
pub const GPIO_R1_SET_REG0: u32 = (GPIO_base + 0xb24);
pub const GPIO_MODE0: u32 = (GPIO_base + 0xc00);
pub const fn GPIO_BANK(reg: u32, pin: u32) -> u32 {
    ((reg) + ((pin) / GPIO_BANK_PINS) * GPIO_BANK_STRIDE)
}
pub const fn GPIO_PIN_MASK(pin: u32) -> u32 {
    (1 << ((pin) % GPIO_BANK_PINS))
}
pub const fn GPIO_MODE_REG(pin: u32) -> u32 {
    (GPIO_MODE0 + ((pin) / GPIO_MODE_ONE_REG_CONTROL_NUM) * GPIO_BANK_STRIDE)
}
pub const fn GPIO_MODE_SHIFT(pin: u32) -> u32 {
    (((pin) % GPIO_MODE_ONE_REG_CONTROL_NUM) * GPIO_MODE_ONE_CONTROL_BITS)
}
pub const GPT1_CTRL: u32 = (GPT_base + 0x00);
pub const GPT1_LEN: u32 = (GPT_base + 0x04);
pub const GPT2_CTRL: u32 = (GPT_base + 0x08);
pub const GPT_STS: u32 = (GPT_base + 0x10);
pub const GPT1_PRESCALER: u32 = (GPT_base + 0x14);
pub const GPT4_CTRL: u32 = GPT_base + 0x28;
pub const GPT4_COUNT: u32 = GPT_base + 0x2c;
pub const GPT4_ENABLE: u32 = 1;
pub const EKD_GPT4_TICKS_PER_MS: u32 = 26_042;
pub const GPT_CTRL_AUTOMODE: u32 = 0x4000;
pub const GPT_CTRL_Enable: u32 = 0x8000;
pub const GPT_STS_1: u32 = 0x0001;
pub const GPT_PRESCALER_div2: u32 = 0x0000;
pub const LCD_STA_REG: u32 = (LCD_base + 0x000);
pub const LCD_INT_ENABLE_REG: u32 = (LCD_base + 0x004);
pub const LCD_INT_STATUS_REG: u32 = (LCD_base + 0x008);
pub const LCD_START_REG: u32 = (LCD_base + 0x00c);
pub const LCD_SIF_PIX_CON_REG: u32 = (LCD_base + 0x018);
pub const LCD_SIF0_TIMING_REG: u32 = (LCD_base + 0x01c);
pub const LCD_SIF_CON_REG: u32 = (LCD_base + 0x028);
pub const LCD_TECON_REG: u32 = (LCD_base + 0x050);
pub const LCD_GMCCON_REG: u32 = (LCD_base + 0x054);
pub const LCD_ROI_CTRL_REG: u32 = (LCD_base + 0x080);
pub const LCD_ROI_OFFSET_REG: u32 = (LCD_base + 0x084);
pub const LCD_ROI_CMD_ADDR_REG: u32 = (LCD_base + 0x088);
pub const LCD_ROI_DATA_ADDR_REG: u32 = (LCD_base + 0x08c);
pub const LCD_ROI_SIZE_REG: u32 = (LCD_base + 0x090);
pub const LCD_ROI_BG_COLOR_REG: u32 = (LCD_base + 0x09c);
pub const LCD_LAYER0_BASE_ADDR: u32 = (LCD_base + 0x0b0);
pub const LCD_LAYER0_CTRL_REG: u32 = (LCD_base + 0x0b0);
pub const LCD_LAYER0_SRC_KEY_REG: u32 = (LCD_base + 0x0b4);
pub const LCD_LAYER0_OFFSET_REG: u32 = (LCD_base + 0x0b8);
pub const LCD_LAYER0_BUFF_ADDR_REG: u32 = (LCD_base + 0x0bc);
pub const LCD_LAYER0_SIZE_REG: u32 = (LCD_base + 0x0c0);
pub const LCD_LAYER0_MEM_OFFSET_REG: u32 = (LCD_base + 0x0c8);
pub const LCD_LAYER0_MEM_PITCH_REG: u32 = (LCD_base + 0x0cc);
pub const LCD_SERIAL_CTRL0_REG: u32 = (LCD_base + 0xf80);
pub const LCD_SERIAL_DATA0_REG: u32 = (LCD_base + 0xf90);
pub const LCD_STATUS_RUN_BIT: u32 = 0x01;
pub const LCD_STATUS_BUSY_BIT: u32 = 0x20;
pub const LCD_START_BIT: u32 = 0x8000;
pub const LCD_INT_ENABLE_COMPLETE_BIT: u32 = 0x01;
pub const LCD_INT_STATUS_COMPLETE_BIT: u32 = 0x01;
pub const LCD_ROI_CTRL_LAYER_MASK: u32 = 0xf0000000;
pub const LCD_GMC_THROTTLE_BURST_MASK: u32 = 0x07;
pub const LCD_SERIAL_CONFIG_SIF0_SIZE_MASK: u32 = 0x07;
pub const LCD_LAYER_CTRL_FORMAT_OFFSET: u32 = 20;
pub const fn LCD_LAYER_ENABLE_BIT(n: u32) -> u32 {
    (1 << (31 - (n)))
}
pub const EKD_LCD_SOFTWARE_TE_REG: u32 = (LCD_base + 0x058);
pub const EKD_LCD_SOFTWARE_TE_VALUE: u32 = 16;
pub const EKD_LCD_SIF0_TIMING: u32 = 0x7700;
pub const EKD_LCD_ROI_FORMAT_RGB565_SERIAL: u32 = 0x10;
pub const EKD_LCD_LAYER_RGB565: u32 = 1;
pub const EKD_LCD_DESTINATION_BIAS: u32 = 1024;
pub const EKD_LCD_DMA_TIMEOUT_TICKS: u32 = 64;
pub const EKD_LCD_FRAME_ALIGNMENT_MASK: u32 = 31;
pub const EKD_LCD_GMC_BURST: u32 = 4;
pub const EKD_LCD_CLOCK_SELECT_MASK: u32 = 0x70;
pub const EKD_LCD_SIF_PIXEL_INIT_CLEAR_MASK: u32 = 0xc0;
pub const EKD_LCD_SIF_INIT_CLEAR_MASK: u32 = 0x0f;
pub const EKD_LCD_SIF_INIT_SET_BITS: u32 = 0x01000000;
pub const EKD_LCD_SIF_TIMING_PRESERVE_MASK: u32 = 0xff000000;
pub const EKD_LCD_PAD_DRIVE_MASK: u32 = 0x00c00000;
pub const EKD_LCD_PAD_DRIVE_VALUE: u32 = 0x00800000;
pub const DCS_READ_DISPLAY_ID: u32 = 0x04;
pub const DCS_SET_COLUMN_ADDRESS: u32 = 0x2a;
pub const DCS_SET_PAGE_ADDRESS: u32 = 0x2b;
pub const DCS_WRITE_MEMORY_START: u32 = 0x2c;
pub const DCS_SET_ADDRESS_MODE: u32 = 0x36;
pub const DCS_MADCTL_MY: u32 = 0x80;
pub const DCS_MADCTL_MX: u32 = 0x40;
pub const DCS_MADCTL_MV: u32 = 0x20;
pub const DCS_MADCTL_ML: u32 = 0x10;
pub const LDO_CON0: u32 = (PMU_base + 0x0c0);
pub const VRF_CON0: u32 = (PMU_base + 0x100);
pub const VRF_CON1: u32 = (PMU_base + 0x104);
pub const VA_CON0: u32 = (PMU_base + 0x120);
pub const VIO28_CON0: u32 = (PMU_base + 0x150);
pub const VUSB_CON0: u32 = (PMU_base + 0x160);
pub const VSIM1_CON0: u32 = (PMU_base + 0x180);
pub const VSIM1_CON1: u32 = (PMU_base + 0x184);
pub const VSIM1_CON2: u32 = (PMU_base + 0x188);
pub const VRTC_CON0: u32 = (PMU_base + 0x1a0);
pub const VIBR_CON0: u32 = (PMU_base + 0x1b0);
pub const VMC_CON0: u32 = (PMU_base + 0x1c0);
pub const VIO18_CON0: u32 = (PMU_base + 0x200);
pub const VCORE_CON3: u32 = (PMU_base + 0x23c);
pub const CHR_CON0: u32 = (PMU_base + 0xa00);
pub const CHR_CON1: u32 = (PMU_base + 0xa04);
pub const CHR_CON4: u32 = (PMU_base + 0xa10);
pub const CHR_CON5: u32 = (PMU_base + 0xa14);
pub const CHR_CON9: u32 = (PMU_base + 0xa24);
pub const CHR_CON10: u32 = (PMU_base + 0xa28);
pub const CHR_CON11: u32 = (PMU_base + 0xa2c);
pub const CHR_CON12: u32 = (PMU_base + 0xa30);
pub const STRUP_CON0: u32 = (PMU_base + 0xa80);
pub const STRUP_CON1: u32 = (PMU_base + 0xa84);
pub const ISINKS_CON0: u32 = (PMU_base + 0xc00);
pub const ISINKS_CON2: u32 = (PMU_base + 0xc08);
pub const ISINK0_CON0: u32 = (PMU_base + 0xc10);
pub const ISINK0_CON1: u32 = (PMU_base + 0xc14);
pub const ISINK1_CON0: u32 = (PMU_base + 0xc20);
pub const ISINK1_CON1: u32 = (PMU_base + 0xc24);
pub const SPK_CON3: u32 = (PMU_base + 0xd0c);
pub const SPK_CON0: u32 = PMU_base + 0xd00;
pub const KPLED_CON0: u32 = PMU_base + 0xc80;
pub const KPLED_MODE_MASK: u32 = 0x0002;
pub const RG_SPK_VOL_MASK: u32 = 0x0030;
pub const RG_SPK_VOL_SHIFT: u32 = 4;
pub const ISINK_STEP_MASK: u32 = 0x0070;
pub const ISINK_STEP_SHIFT: u32 = 4;
pub const SPK_CON6: u32 = (PMU_base + 0xd18);
pub const TEST_CON0: u32 = (PMU_base + 0xf00);
pub const TEST_CON1: u32 = (PMU_base + 0xf04);
pub const fn PMU_REG_OFFSET(reg: u32) -> u32 {
    ((reg) - PMU_base)
}
pub const EKD_PMU_TRIM_BANK_220: u32 = 0x220;
pub const LDO_CAL_MASK: u32 = 0x00f0;
pub const LDO_CAL_SHIFT: u32 = 4;
pub const EKD_PMU_PWKEY_LONG_PRESS_EN: u32 = 0x01;
pub const EKD_PMU_PWKEY_SHUTDOWN_SEL: u32 = 0x40;
pub const EKD_PMU_PWKEY_TRIGGER: u32 = 0x10;
pub const ISINK_EN_MASK: u32 = 0x01;
pub const EKD_ISINK_CONFIG_MASK: u32 = 0x0171;
pub const EKD_ISINK_24MA_PWM: u32 = 0x50;
pub const ISINKS_MODE_MASK: u32 = 0x01;
pub const EKD_ADIE_CLOCK_STATUS: u32 = (ANA_CFGSYS_base + 0x008);
pub const EKD_ADIE_CLOCK_CLEAR: u32 = (ANA_CFGSYS_base + 0x010);
pub const EKD_ADIE_ISINK_26M: u32 = 0x10;
pub const EKD_ADIE_POWER_KEY_STATUS: u32 = (ANA_CFGSYS_base + 0x048);
pub const EKD_ADIE_POWER_KEY_RELEASED: u32 = 0x01;
pub const EKD_BOOT_FLAGS: u32 = (CONFIG_base + 0x010);
pub const EKD_BOOT_USB_WITHOUT_BATTERY: u32 = 0x08;
pub const EKD_PMU_EFUSE: u32 = (EFUSE_base + 0x10c);
pub const EKD_EFUSE_CV_VALID_SHIFT: u32 = 12;
pub const EKD_EFUSE_CV_CODE_SHIFT: u32 = 13;
pub const EKD_EFUSE_CV_CODE_MASK: u32 = 31;
pub const RG_VIBR_STB_SEL_MASK: u32 = 0x0800;
pub const RG_VIBR_STB_SEL_SHIFT: u32 = 11;
pub const RG_VMC_STB_SEL_MASK: u32 = 0x0800;
pub const RG_VMC_STB_SEL_SHIFT: u32 = 11;
pub const VCORE_VOSEL_SLEEP_MASK: u32 = 0x01F0;
pub const VCORE_VOSEL_SLEEP_SHIFT: u32 = 4;
pub const VCORE_SFSTREN_MASK: u32 = 0x0008;
pub const VCORE_SFSTREN_SHIFT: u32 = 3;
pub const RG_VCDT_HV_VTH_MASK: u32 = 0xF000;
pub const RG_VCDT_HV_VTH_SHIFT: u32 = 12;
pub const RG_VCDT_HV_EN_MASK: u32 = 0x0001;
pub const RG_VCDT_HV_EN_SHIFT: u32 = 0;
pub const RG_VBAT_CC_VTH_MASK: u32 = 0x0030;
pub const RG_VBAT_CC_VTH_SHIFT: u32 = 4;
pub const RG_CSDAC_STP_MASK: u32 = 0x0700;
pub const RG_CSDAC_STP_SHIFT: u32 = 8;
pub const RG_BATON_TDET_EN_MASK: u32 = 0x1000;
pub const RG_BATON_TDET_EN_SHIFT: u32 = 12;
pub const RG_VBAT_OV_VTH_MASK: u32 = 0x0070;
pub const RG_VBAT_OV_VTH_SHIFT: u32 = 4;
pub const RG_CHRWDT_TD_MASK: u32 = 0x000F;
pub const RG_CHRWDT_TD_SHIFT: u32 = 0;
pub const RG_ADCIN_CHR_EN_MASK: u32 = 0x0040;
pub const RG_ADCIN_CHR_EN_SHIFT: u32 = 6;
pub const RG_ADCIN_VSEN_EN_MASK: u32 = 0x0020;
pub const RG_ADCIN_VSEN_EN_SHIFT: u32 = 5;
pub const RG_ADCIN_VBAT_EN_MASK: u32 = 0x0010;
pub const RG_ADCIN_VBAT_EN_SHIFT: u32 = 4;
pub const RG_BC11_RST_MASK: u32 = 0x0800;
pub const RG_BC11_RST_SHIFT: u32 = 11;
pub const RG_BC11_BB_CTRL_MASK: u32 = 0x0400;
pub const RG_BC11_BB_CTRL_SHIFT: u32 = 10;
pub const RG_ULC_DET_EN_MASK: u32 = 0x0080;
pub const RG_ULC_DET_EN_SHIFT: u32 = 7;
pub const RG_HWCV_EN_MASK: u32 = 0x0040;
pub const RG_HWCV_EN_SHIFT: u32 = 6;
pub const RG_CSDAC_MODE_MASK: u32 = 0x0004;
pub const RG_CSDAC_MODE_SHIFT: u32 = 2;
pub const THR_HWPDN_EN_MASK: u32 = 0x0008;
pub const THR_HWPDN_EN_SHIFT: u32 = 3;
pub const RG_PMU_PGDET_DIS_MASK: u32 = 0x0008;
pub const RG_PMU_PGDET_DIS_SHIFT: u32 = 3;
pub const RG_SPK_IN_FLOAT_B_MASK: u32 = 0x4000;
pub const RG_SPK_IN_FLOAT_B_SHIFT: u32 = 14;
pub const RG_SPK_OUT_FLOAT_B_MASK: u32 = 0x0800;
pub const RG_SPK_OUT_FLOAT_B_SHIFT: u32 = 11;
pub const RG_SPK_RSV_MASK: u32 = 0x0F00;
pub const RG_SPK_RSV_SHIFT: u32 = 8;
pub const RG_PWRKEY_RST_EN_MASK: u32 = 0x0001;
pub const RG_PWRKEY_RST_EN_SHIFT: u32 = 0;
pub const RG_LNGP_SHUTDOWN_SEL_MASK: u32 = 0x0040;
pub const RG_LNGP_SHUTDOWN_SEL_SHIFT: u32 = 6;
pub const RG_PWRKEY_RST_FUNC_SET_MASK: u32 = 0x0010;
pub const RG_PWRKEY_RST_FUNC_SET_SHIFT: u32 = 4;
pub const RG_IBIAS_TRIM_MASK: u32 = 0x0F00;
pub const RG_IBIAS_TRIM_SHIFT: u32 = 8;
pub const VSIM1_GPLDO_EN_MASK: u32 = 0x0002;
pub const VSIM1_GPLDO_EN_SHIFT: u32 = 1;
pub const RG_VSIM1_EN_MASK: u32 = 0x0001;
pub const RG_VSIM1_EN_SHIFT: u32 = 0;
pub const LDO_BUCK_VOL_SEL_MASK: u32 = 0x01f0;
pub const EKD_LDO_CON0_STARTUP_BIT: u32 = 0x0001;
pub const PWM4_CTRL: u32 = (PWM2_base + 0x00);
pub const PWM4_COUNT: u32 = (PWM2_base + 0x04);
pub const PWM4_DUTY: u32 = (PWM2_base + 0x08);
pub const EKD_PWM3_CTRL: u32 = (PWM_2CH_base + 0x18);
pub const EKD_PWM3_CLOCK_QUERY_DEFAULT: u32 = 7;
pub const EKD_PWM4_DIV8_CONTROL: u32 = 3;
pub const EKD_PWM4_20KHZ_COUNT: u32 = 80;
pub const EKD_PWM4_DEFAULT_DUTY: u32 = 29;
pub const EKD_PWM4_DUTY_STEPS: u32 = 81;
pub const EKD_PWM4_BRIGHTNESS_STEP_PERCENT: u32 = 12;
pub const RTC_BBPU: u32 = (RTC_base + 0x00);
pub const RTC_TC_SEC: u32 = (RTC_base + 0x14);
pub const RTC_TC_MIN: u32 = (RTC_base + 0x18);
pub const RTC_TC_HOU: u32 = (RTC_base + 0x1c);
pub const RTC_XOSCCAL: u32 = (RTC_base + 0x4c);
pub const RTC_PROT: u32 = (RTC_base + 0x68);
pub const RTC_WRTGR: u32 = (RTC_base + 0x74);
pub const RTC_PROTECT1: u32 = 0x586a;
pub const RTC_PROTECT2: u32 = 0x9136;
pub const RTC_WRTGR_WRTGR: u32 = 0x01;
pub const EKD_RTC_CBUSY: u32 = 0x40;
pub const EKD_RTC_BBPU_POWER_LATCH: u32 = 0x04;
pub const EKD_RTC_RELOAD_KEY: u32 = 0x4320;
pub const EKD_RTC_RELEASE_KEY: u32 = 0x4308;
pub const EKD_RTC_OSC_EMBEDDED: u32 = 0x20;
pub const EKD_RTC_OSC_MODE_MASK: u32 = 0x1c0;
pub const EKD_RTC_OSC_SHUTDOWN_MODE: u32 = 0x140;
pub const EKD_RTC_OSC_UNLOCK1: u32 = 0x1a57;
pub const EKD_RTC_OSC_UNLOCK2: u32 = 0x2b68;
pub const RTC_MINUTE_MASK: u32 = 0x3f;
pub const RTC_HOUR_MASK: u32 = 0x1f;
pub const RW_SFI_MAC_CTL: u32 = (SFI_base + 0x000);
pub const RW_SFI_DIRECT_CTL: u32 = (SFI_base + 0x004);
pub const RW_SFI_MISC_CTL: u32 = (SFI_base + 0x008);
pub const RW_SFI_MAC_OUTL: u32 = (SFI_base + 0x010);
pub const RW_SFI_MAC_INL: u32 = (SFI_base + 0x014);
pub const RW_SFI_MISC_CTL3: u32 = (SFI_base + 0x044);
pub const RW_SFI_GPRAM_DATA: u32 = (SFI_base + 0x800);
pub const SFI_WIP: u32 = 0x01;
pub const SFI_WIP_READY: u32 = 0x02;
pub const SFI_TRIG: u32 = 0x04;
pub const SFI_MAC_EN: u32 = 0x08;
pub const SFI_MAC_SIO_SEL: u32 = 0x10;
pub const SFI_MAC_SEL: u32 = 0x10000000;
pub const SFI_QPI_EN: u32 = 0x01;
pub const SFI_REQ_IDLE: u32 = 0x100;
pub const SFI_CH2_TRANS_MASK: u32 = 0x200;
pub const SFI_CH2_TRANS_IDLE: u32 = 0x2000;
pub const SFI_TRANSFER_STATUS_MASK: u32 = (SFI_WIP | SFI_WIP_READY);
pub const SFI_MANUAL_MODE_CLEAR_MASK: u32 =
    (SFI_MAC_SEL | SFI_MAC_SIO_SEL | SFI_MAC_EN | SFI_TRIG | SFI_TRANSFER_STATUS_MASK);
pub const SFI_TRIGGER_CLEAR_MASK: u32 = (SFI_MAC_SEL | SFI_MAC_SIO_SEL | SFI_TRIG);
pub const NOR_CMD_RDID: u32 = 0x9f;
pub const NOR_CMD_RDSR1: u32 = 0x05;
pub const NOR_CMD_RDSR2: u32 = 0x35;
pub const NOR_CMD_WREN: u32 = 0x06;
pub const NOR_CMD_WRDI: u32 = 0x04;
pub const NOR_CMD_PAGE_PROGRAM: u32 = 0x02;
pub const NOR_CMD_SECTOR_ERASE: u32 = 0x20;
pub const NOR_SR1_WIP: u32 = 0x01;
pub const NOR_SR1_WEL: u32 = 0x02;
pub const NOR_SR2_SUSPEND_MASK: u32 = 0x84;
pub const NOR_JEDEC_GD25LQ64: u32 = 0xc86017;
pub const EKD_NOR_BASE: u32 = 0x10000000;
pub const EKD_NOR_BYTES: u32 = 0x800000;
pub const CLK_CONDA: u32 = (CONFIG_base + 0x100);
pub const CLK_CONDC: u32 = (CONFIG_base + 0x108);
pub const CLK_CONDH: u32 = (CONFIG_base + 0x11c);
pub const DRVPDN_CON2: u32 = (CONFIG_base + 0x308);
pub const DRVPDN_CON0_SET: u32 = (CONFIG_base + 0x310);
pub const DRVPDN_CON2_SET: u32 = (CONFIG_base + 0x318);
pub const DRVPDN_CON0_CLR: u32 = (CONFIG_base + 0x320);
pub const DRVPDN_CON1_CLR: u32 = (CONFIG_base + 0x324);
pub const DRVPDN_CON2_CLR: u32 = (CONFIG_base + 0x328);
pub const MPLL_CON0: u32 = (MIXED_base + 0x100);
pub const MPLL_CON1: u32 = (MIXED_base + 0x104);
pub const WDT_RESTART: u32 = (RGU_base + 0x008);
pub const WDT_RESTART_KEY: u32 = 0x1971;
pub const EMI_GENA: u32 = (EMI_base + 0x070);
pub const EKD_PDN_LCD_CLOCKS: u32 = 0x00c1;
pub const EKD_PDN_UART3: u32 = 0x8000;
pub const EKD_PDN_BT_CLOCKS: u32 = 0xa200;
pub const EKD_PDN_BT_ARM7: u32 = 0x8000;
pub const EKD_PDN_BT_BUS_26M: u32 = 0x2200;
pub const EKD_PDN_BTIF: u32 = 0x0002;
pub const EKD_PDN_PWM4: u32 = 0x0010;
pub const EKD_BT_POWER_CON1: u32 = 0xa0180804;
pub const EKD_BT_POWER_ON: u32 = 0x0044;
pub const EKD_BT_POWER_OFF_MASK: u32 = 0x2044;
pub const fn UART_RBR(base: u32) -> u32 {
    ((base) + 0x00)
}
pub const fn UART_THR(base: u32) -> u32 {
    ((base) + 0x00)
}
pub const fn UART_DLL(base: u32) -> u32 {
    ((base) + 0x00)
}
pub const fn UART_IER(base: u32) -> u32 {
    ((base) + 0x04)
}
pub const fn UART_DLH(base: u32) -> u32 {
    ((base) + 0x04)
}
pub const fn UART_IIR(base: u32) -> u32 {
    ((base) + 0x08)
}
pub const fn UART_FCR(base: u32) -> u32 {
    ((base) + 0x08)
}
pub const fn UART_EFR(base: u32) -> u32 {
    ((base) + 0x08)
}
pub const fn UART_LCR(base: u32) -> u32 {
    ((base) + 0x0c)
}
pub const fn UART_MCR(base: u32) -> u32 {
    ((base) + 0x10)
}
pub const fn UART_LSR(base: u32) -> u32 {
    ((base) + 0x14)
}
pub const fn UART_AUTOBAUD_EN(base: u32) -> u32 {
    ((base) + 0x20)
}
pub const fn UART_RATE_STEP(base: u32) -> u32 {
    ((base) + 0x24)
}
pub const fn UART_STEP_COUNT(base: u32) -> u32 {
    ((base) + 0x28)
}
pub const fn UART_SAMPLE_COUNT(base: u32) -> u32 {
    ((base) + 0x2c)
}
pub const fn UART_RATE_FIX_REG(base: u32) -> u32 {
    ((base) + 0x34)
}
pub const fn UART_ESCAPE_EN(base: u32) -> u32 {
    ((base) + 0x44)
}
pub const fn UART_SLEEP_EN(base: u32) -> u32 {
    ((base) + 0x48)
}
pub const fn UART_RXDMA(base: u32) -> u32 {
    ((base) + 0x4c)
}
pub const fn UART_FRACDIV_L(base: u32) -> u32 {
    ((base) + 0x54)
}
pub const fn UART_FRACDIV_M(base: u32) -> u32 {
    ((base) + 0x58)
}
pub const UART_LCR_DLAB: u32 = 0x80;
pub const UART_LCR_8N1: u32 = 0x03;
pub const UART_LCR_ENHANCED_ACCESS: u32 = 0xbf;
pub const UART_EFR_EN: u32 = 0x10;
pub const UART_IER_ERBFI: u32 = 0x01;
pub const UART_IER_ELSI: u32 = 0x04;
pub const UART_FCR_FIFOINI: u32 = 0x07;
pub const UART_LSR_DR: u32 = 0x01;
pub const UART_LSR_OE: u32 = 0x02;
pub const UART_LSR_PE: u32 = 0x04;
pub const UART_LSR_FE: u32 = 0x08;
pub const UART_LSR_BI: u32 = 0x10;
pub const UART_LSR_THRE: u32 = 0x20;
pub const UART_LSR_TEMT: u32 = 0x40;
pub const UART_LSR_ERROR_MASK: u32 = (UART_LSR_OE | UART_LSR_PE | UART_LSR_FE | UART_LSR_BI);
