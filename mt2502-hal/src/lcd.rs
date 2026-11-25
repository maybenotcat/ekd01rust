//! MT2502 serial LCD bus and RGB565 DMA mechanisms. No panel or board policy.
use crate::{mmio::*, registers::*};
pub struct BusConfiguration {
    pub gmc_burst: u32,
    pub software_te: u32,
    pub clock_selection: u32,
    pub pixel_clear: u32,
    pub serial_clear: u32,
    pub serial_bits: u32,
    pub timing_preserve: u32,
    pub timing: u32,
    pub drive_mask: u32,
    pub drive_bits: u32,
}
pub struct Lcd;
impl Lcd {
    pub fn enable_clock() {
        write32(DRVPDN_CON0_CLR, EKD_PDN_LCD_CLOCKS);
    }
    pub fn configure_bus(configuration: &BusConfiguration) {
        update32(
            LCD_GMCCON_REG,
            LCD_GMC_THROTTLE_BURST_MASK,
            configuration.gmc_burst,
        );
        write32(EKD_LCD_SOFTWARE_TE_REG, configuration.software_te);
        update32(
            CLK_CONDH,
            EKD_LCD_CLOCK_SELECT_MASK,
            configuration.clock_selection,
        );
        update16(LCD_SIF_PIX_CON_REG, configuration.pixel_clear, 0);
        update32(
            LCD_SIF_CON_REG,
            configuration.serial_clear,
            configuration.serial_bits,
        );
        write32(
            LCD_SIF0_TIMING_REG,
            (read32(LCD_SIF0_TIMING_REG) & configuration.timing_preserve) | configuration.timing,
        );
        update32(
            EKD_GPIO_DRV_BANK1,
            configuration.drive_mask,
            configuration.drive_bits,
        );
    }
    pub fn settle() {
        for _ in 0..20 {
            unsafe {
                core::arch::asm!("nop", options(nomem, nostack));
            }
        }
    }
    pub fn write_serial(data: bool, value: u16, width: u8) {
        update32(LCD_SIF_CON_REG, LCD_SERIAL_CONFIG_SIF0_SIZE_MASK, 0);
        let register = if data {
            LCD_SERIAL_DATA0_REG
        } else {
            LCD_SERIAL_CTRL0_REG
        };
        if width == 1 {
            write8(register, value as u8);
        } else {
            write16(register, value);
        }
        Self::settle();
    }
    pub fn repeat(value: u8, count: u32) {
        update32(LCD_SIF_CON_REG, LCD_SERIAL_CONFIG_SIF0_SIZE_MASK, 0);
        for _ in 0..count {
            write8(LCD_SERIAL_DATA0_REG, value);
        }
        Self::settle();
    }
    pub fn read_byte() -> u8 {
        read8(LCD_SERIAL_DATA0_REG)
    }
    pub fn stop_dma() {
        write16(LCD_START_REG, 0);
        update32(LCD_ROI_CTRL_REG, LCD_ROI_CTRL_LAYER_MASK, 0);
        write16(LCD_INT_ENABLE_REG, 0);
    }
    pub fn status() -> u32 {
        read16(LCD_STA_REG) as u32
    }
    pub fn completion_pending() -> bool {
        read16(LCD_INT_STATUS_REG) as u32 & LCD_INT_STATUS_COMPLETE_BIT != 0
    }
    pub fn prepare_dma() {
        write32(DRVPDN_CON0_CLR, EKD_PDN_LCD_CLOCKS);
        write32(IRQ_MASK_SET0, 1 << EKD_IRQ_LCD_LINE);
        write16(LCD_START_REG, 0);
        write16(LCD_INT_ENABLE_REG, LCD_INT_ENABLE_COMPLETE_BIT as u16);
        let _ = read16(LCD_INT_STATUS_REG);
        write32(LCD_TECON_REG, 0);
    }
    pub fn configure_rgb565(address: u32, width: u32, height: u32, region: [u16; 4], bias: u32) {
        let [x0, y0, x1, y1] = region;
        write32(
            LCD_ROI_CTRL_REG,
            LCD_LAYER_ENABLE_BIT(0) | EKD_LCD_ROI_FORMAT_RGB565_SERIAL,
        );
        write32(
            LCD_ROI_OFFSET_REG,
            ((y0 as u32 + bias) << 16) | (x0 as u32 + bias),
        );
        write32(LCD_ROI_CMD_ADDR_REG, LCD_SERIAL_CTRL0_REG);
        write32(LCD_ROI_DATA_ADDR_REG, LCD_SERIAL_DATA0_REG);
        write32(
            LCD_ROI_SIZE_REG,
            ((y1 - y0 + 1) as u32) << 16 | (x1 - x0 + 1) as u32,
        );
        write32(LCD_ROI_BG_COLOR_REG, 0);
        for offset in (0..32).step_by(4) {
            write32(LCD_LAYER0_BASE_ADDR + offset, 0);
        }
        write32(
            LCD_LAYER0_CTRL_REG,
            EKD_LCD_LAYER_RGB565 << LCD_LAYER_CTRL_FORMAT_OFFSET,
        );
        write32(LCD_LAYER0_SRC_KEY_REG, 0);
        write32(LCD_LAYER0_OFFSET_REG, (bias << 16) | bias);
        write32(LCD_LAYER0_BUFF_ADDR_REG, address);
        write32(LCD_LAYER0_SIZE_REG, height << 16 | width);
        write32(LCD_LAYER0_MEM_OFFSET_REG, 0);
        write32(LCD_LAYER0_MEM_PITCH_REG, width * 2);
    }
    /// # Safety
    /// The readable framebuffer must contain at least count pixels, count > 0,
    /// and remain accessible for DMA. The board validates its memory placement.
    pub unsafe fn flush_framebuffer(pointer: *const u16, count: usize) {
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
        unsafe {
            core::ptr::read_volatile(pointer.add(count - 1));
        }
        let _ = read32(LCD_LAYER0_BUFF_ADDR_REG);
    }
    pub fn clear_pending() {
        let _ = read16(LCD_INT_STATUS_REG);
    }
    pub fn start() {
        write16(LCD_START_REG, 0);
        write16(LCD_START_REG, LCD_START_BIT as u16);
        let _ = read16(LCD_START_REG);
    }
    pub fn acknowledge_interrupt() {
        write32(IRQ_EOIOH0, 1 << EKD_IRQ_LCD_LINE);
    }
}
