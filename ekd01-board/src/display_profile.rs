//! N22 serial panel bus selections.
pub const EKD_LCD_SOFTWARE_TE_VALUE: u32 = 16;
pub const EKD_LCD_SIF0_TIMING: u32 = 0x7700;
pub const EKD_LCD_GMC_BURST: u32 = 4;
pub const EKD_LCD_SIF_PIXEL_INIT_CLEAR_MASK: u32 = 0xc0;
pub const EKD_LCD_SIF_INIT_CLEAR_MASK: u32 = 0x0f;
pub const EKD_LCD_SIF_INIT_SET_BITS: u32 = 0x01000000;
pub const EKD_LCD_SIF_TIMING_PRESERVE_MASK: u32 = 0xff000000;
pub const EKD_LCD_PAD_DRIVE_MASK: u32 = 0x00c00000;
pub const EKD_LCD_PAD_DRIVE_VALUE: u32 = 0x00800000;
pub const EKD_LCD_DMA_TIMEOUT_TICKS: u32 = 64;

use mt2502_hal::lcd::BusConfiguration;
pub const BUS: BusConfiguration = BusConfiguration {
    gmc_burst: EKD_LCD_GMC_BURST,
    software_te: EKD_LCD_SOFTWARE_TE_VALUE,
    clock_selection: 0,
    pixel_clear: EKD_LCD_SIF_PIXEL_INIT_CLEAR_MASK,
    serial_clear: EKD_LCD_SIF_INIT_CLEAR_MASK,
    serial_bits: EKD_LCD_SIF_INIT_SET_BITS,
    timing_preserve: EKD_LCD_SIF_TIMING_PRESERVE_MASK,
    timing: EKD_LCD_SIF0_TIMING,
    drive_mask: EKD_LCD_PAD_DRIVE_MASK,
    drive_bits: EKD_LCD_PAD_DRIVE_VALUE,
};
