use super::{DebugUart, gpio::Pin, mmio::*, panels::PANELS, pmu::Pmu, registers::*, time::Timer};
use crate::board::pins;
pub const WIDTH: usize = 320;
pub const HEIGHT: usize = 240;

#[derive(Debug)]
pub enum DisplayError {
    UnknownPanel(u32),
    Framebuffer,
    DmaIdle,
    DmaTimeout,
    Disabled,
}
pub struct Display {
    reset: Pin,
    cs: Pin,
    clock: Pin,
    _bus: [Pin; 3],
    failed: bool,
    first_transfer: bool,
}
impl Display {
    pub async fn initialize(pmu: &mut Pmu) -> Result<Self, DisplayError> {
        DebugUart::log("[LCD] INIT_ENTER\n");
        let mut display = unsafe {
            Self {
                reset: Pin::claim(pins::LCD_RESET),
                cs: Pin::claim(pins::LCD_CS),
                clock: Pin::claim(pins::LCD_CLOCK),
                _bus: pins::LCD_OTHER_BUS.map(|number| Pin::claim(number)),
                failed: false,
                first_transfer: true,
            }
        };
        for pin in [&mut display.reset, &mut display.cs] {
            pin.set(true);
            pin.output();
            pin.mode(0);
            pin.disable_pull();
        }
        for pin in &mut display._bus {
            pin.output();
            pin.disable_pull();
            pin.mode(1);
        }
        display.clock.output();
        display.clock.disable_pull();
        display.clock.mode(1);
        write32(DRVPDN_CON0_CLR, EKD_PDN_LCD_CLOCKS);
        pmu.enable_lcd_supply();
        Timer::delay_ms(20).await;
        update32(
            LCD_GMCCON_REG,
            LCD_GMC_THROTTLE_BURST_MASK,
            EKD_LCD_GMC_BURST,
        );
        write32(EKD_LCD_SOFTWARE_TE_REG, EKD_LCD_SOFTWARE_TE_VALUE);
        update32(CLK_CONDH, EKD_LCD_CLOCK_SELECT_MASK, 0);
        update16(LCD_SIF_PIX_CON_REG, EKD_LCD_SIF_PIXEL_INIT_CLEAR_MASK, 0);
        update32(
            LCD_SIF_CON_REG,
            EKD_LCD_SIF_INIT_CLEAR_MASK,
            EKD_LCD_SIF_INIT_SET_BITS,
        );
        write32(
            LCD_SIF0_TIMING_REG,
            (read32(LCD_SIF0_TIMING_REG) & EKD_LCD_SIF_TIMING_PRESERVE_MASK) | EKD_LCD_SIF0_TIMING,
        );
        update32(
            EKD_GPIO_DRV_BANK1,
            EKD_LCD_PAD_DRIVE_MASK,
            EKD_LCD_PAD_DRIVE_VALUE,
        );
        let id = display.read_id().await;
        DebugUart::hex("[LCD] ID", id);
        let middle = (id >> 8) & 0xffff;
        let first = id >> 16;
        let profile = if middle == 0x8552 {
            0
        } else if middle == 0x8901 {
            1
        } else if middle == 0x81b3 && id >> 24 != 1 {
            2
        } else if first == 0x0181 {
            3
        } else if first == 0x7789 && (id >> 8) & 255 == 0x52 {
            4
        } else if middle == 0xffff || middle == 0x8966 {
            5
        } else {
            return Err(DisplayError::UnknownPanel(id));
        };
        DebugUart::log(PANELS[profile].name);
        DebugUart::log("\n");
        let mut command = 0;
        let mut native = 0xd0u8;
        for &(op, width, value, count) in PANELS[profile].ops {
            if op == 0 {
                command = value;
            }
            if op == 1 && command == DCS_SET_ADDRESS_MODE as u16 {
                native = value as u8;
            }
            match op {
                0 | 1 => {
                    if op == 1 && width == 1 && count >= (WIDTH * HEIGHT * 2) as u32 {
                        display.repeat(value as u8, count);
                    } else {
                        for _ in 0..count {
                            display.bus_write(op != 0, value, width);
                        }
                    }
                }
                2 => display.reset.set(value != 0),
                3 => display.cs.set(value != 0),
                4 => Timer::delay_ms(value as u64).await,
                5 => {
                    display.window(0, 0, 239, 319);
                    display.cs.set(false);
                    display.repeat(0, (WIDTH * HEIGHT * 2) as u32);
                    display.cs.set(true);
                }
                _ => unreachable!(),
            }
        }
        let orientation = if native as u32 & DCS_MADCTL_MV != 0 {
            native
        } else {
            (native & 0x1f)
                | DCS_MADCTL_MV as u8
                | if native as u32 & DCS_MADCTL_MX == 0 {
                    DCS_MADCTL_MY as u8
                } else {
                    0
                }
                | if native as u32 & DCS_MADCTL_MY != 0 {
                    DCS_MADCTL_MX as u8
                } else {
                    0
                }
        };
        display.command(DCS_SET_ADDRESS_MODE as u8);
        display.datum(orientation);
        display.window(0, 0, 319, 239);
        DebugUart::hex("[LCD] LANDSCAPE_MADCTL", orientation as u32);
        DebugUart::log("[LCD] PANEL_INIT_EXIT\n");
        Ok(display)
    }
    fn settle() {
        for _ in 0..20 {
            unsafe {
                core::arch::asm!("nop", options(nomem, nostack));
            }
        }
    }
    fn bus_write(&mut self, data: bool, value: u16, width: u8) {
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
    fn transfer(&mut self, data: bool, value: u8) {
        self.cs.set(false);
        self.bus_write(data, value as u16, 2);
        self.cs.set(true);
    }
    fn command(&mut self, value: u8) {
        self.transfer(false, value);
    }
    fn datum(&mut self, value: u8) {
        self.transfer(true, value);
    }
    fn repeat(&mut self, value: u8, count: u32) {
        update32(LCD_SIF_CON_REG, LCD_SERIAL_CONFIG_SIF0_SIZE_MASK, 0);
        for _ in 0..count {
            write8(LCD_SERIAL_DATA0_REG, value);
        }
        Self::settle();
    }
    async fn read_id(&mut self) -> u32 {
        self.reset.set(true);
        Timer::delay_ms(200).await;
        self.reset.set(false);
        Timer::delay_ms(10).await;
        self.reset.set(true);
        Timer::delay_ms(120).await;
        self.cs.set(false);
        self.bus_write(false, DCS_READ_DISPLAY_ID as u16, 2);
        self.clock.mode(0);
        self.clock.output();
        self.clock.set(false);
        self.clock.set(true);
        self.clock.mode(1);
        let mut id = 0;
        for _ in 0..4 {
            id = (id << 8) | read8(LCD_SERIAL_DATA0_REG) as u32;
        }
        self.cs.set(true);
        Timer::delay_ms(1).await;
        id
    }
    fn window(&mut self, x0: u16, y0: u16, x1: u16, y1: u16) {
        self.command(DCS_SET_COLUMN_ADDRESS as u8);
        for value in [x0, x1] {
            for byte in value.to_be_bytes() {
                self.datum(byte);
            }
        }
        self.command(DCS_SET_PAGE_ADDRESS as u8);
        for value in [y0, y1] {
            for byte in value.to_be_bytes() {
                self.datum(byte);
            }
        }
        self.command(DCS_WRITE_MEMORY_START as u8);
    }
    fn stop_dma(&mut self) {
        write16(LCD_START_REG, 0);
        update32(LCD_ROI_CTRL_REG, LCD_ROI_CTRL_LAYER_MASK, 0);
        write16(LCD_INT_ENABLE_REG, 0);
        self.cs.set(true);
    }
    fn fail(&mut self, error: DisplayError) -> Result<(), DisplayError> {
        self.stop_dma();
        self.failed = true;
        DebugUart::hex("[LCD] DMA_STATUS", read16(LCD_STA_REG) as u32);
        Err(error)
    }
    pub async fn present(&mut self, pixels: &[u16], region: [u16; 4]) -> Result<(), DisplayError> {
        if self.failed {
            return Err(DisplayError::Disabled);
        }
        let [x0, y0, x1, y1] = region;
        let address = pixels.as_ptr() as u32;
        let trace = core::mem::replace(&mut self.first_transfer, false);
        if trace {
            let cpsr: u32;
            unsafe {
                core::arch::asm!("mrs {value}, cpsr", value=out(reg) cpsr, options(nomem, nostack, preserves_flags));
            }
            DebugUart::hex("[LCD] DMA_ENTER_CPSR", cpsr);
            DebugUart::hex("[LCD] DMA_ENTER_STATUS", read16(LCD_STA_REG) as u32);
            DebugUart::hex("[LCD] DMA_REGION_BEGIN", (y0 as u32) << 16 | x0 as u32);
            DebugUart::hex("[LCD] DMA_REGION_END", (y1 as u32) << 16 | x1 as u32);
        }
        if pixels.len() != WIDTH * HEIGHT
            || address & EKD_LCD_FRAME_ALIGNMENT_MASK != 0
            || address < 0x100000
            || address > 0x700000 - (WIDTH * HEIGHT * 2) as u32
            || x0 > x1
            || y0 > y1
            || x1 >= WIDTH as u16
            || y1 >= HEIGHT as u16
        {
            DebugUart::hex("[LCD] FRAMEBUFFER_ADDRESS", address);
            DebugUart::hex("[LCD] FRAMEBUFFER_PIXELS", pixels.len() as u32);
            DebugUart::hex("[LCD] REGION_BEGIN", (y0 as u32) << 16 | x0 as u32);
            DebugUart::hex("[LCD] REGION_END", (y1 as u32) << 16 | x1 as u32);
            return self.fail(DisplayError::Framebuffer);
        }
        let started = Timer::now();
        if trace {
            DebugUart::hex("[LCD] DMA_IDLE_WAIT_TICK", started);
        }
        while read16(LCD_STA_REG) as u32 & (LCD_STATUS_RUN_BIT | LCD_STATUS_BUSY_BIT) != 0 {
            if Timer::now().wrapping_sub(started) >= EKD_LCD_DMA_TIMEOUT_TICKS {
                return self.fail(DisplayError::DmaIdle);
            }
            embassy_time::Timer::after_ticks(1).await;
        }
        if trace {
            DebugUart::log("[LCD] DMA_IDLE_READY\n");
        }
        write32(DRVPDN_CON0_CLR, EKD_PDN_LCD_CLOCKS);
        write32(IRQ_MASK_SET0, 1 << EKD_IRQ_LCD_LINE);
        write16(LCD_START_REG, 0);
        write16(LCD_INT_ENABLE_REG, LCD_INT_ENABLE_COMPLETE_BIT as u16);
        let _ = read16(LCD_INT_STATUS_REG);
        write32(LCD_TECON_REG, 0);
        self.window(x0, y0, x1, y1);
        if trace {
            DebugUart::log("[LCD] DMA_WINDOW_READY\n");
        }
        write32(
            LCD_ROI_CTRL_REG,
            LCD_LAYER_ENABLE_BIT(0) | EKD_LCD_ROI_FORMAT_RGB565_SERIAL,
        );
        write32(
            LCD_ROI_OFFSET_REG,
            ((y0 as u32 + EKD_LCD_DESTINATION_BIAS) << 16) | (x0 as u32 + EKD_LCD_DESTINATION_BIAS),
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
        write32(
            LCD_LAYER0_OFFSET_REG,
            (EKD_LCD_DESTINATION_BIAS << 16) | EKD_LCD_DESTINATION_BIAS,
        );
        write32(LCD_LAYER0_BUFF_ADDR_REG, address);
        write32(LCD_LAYER0_SIZE_REG, (HEIGHT as u32) << 16 | WIDTH as u32);
        write32(LCD_LAYER0_MEM_OFFSET_REG, 0);
        write32(LCD_LAYER0_MEM_PITCH_REG, (WIDTH * 2) as u32);
        if trace {
            DebugUart::log("[LCD] DMA_REGISTERS_READY\n");
        }
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
        unsafe {
            core::ptr::read_volatile(pixels.as_ptr().add(WIDTH * HEIGHT - 1));
        }
        let _ = read32(LCD_LAYER0_BUFF_ADDR_REG);
        self.cs.set(false);
        let _ = read16(LCD_INT_STATUS_REG);
        let started = Timer::now();
        write16(LCD_START_REG, 0);
        write16(LCD_START_REG, LCD_START_BIT as u16);
        let _ = read16(LCD_START_REG);
        if trace {
            DebugUart::hex("[LCD] DMA_STARTED_STATUS", read16(LCD_STA_REG) as u32);
            DebugUart::hex("[LCD] DMA_STARTED_TICK", Timer::now());
        }
        let mut saw_run = false;
        let mut complete = false;
        loop {
            let running =
                read16(LCD_STA_REG) as u32 & (LCD_STATUS_RUN_BIT | LCD_STATUS_BUSY_BIT) != 0;
            complete |= read16(LCD_INT_STATUS_REG) as u32 & LCD_INT_STATUS_COMPLETE_BIT != 0;
            saw_run |= running;
            if (complete || saw_run) && !running {
                break;
            }
            if Timer::now().wrapping_sub(started) >= EKD_LCD_DMA_TIMEOUT_TICKS {
                return self.fail(DisplayError::DmaTimeout);
            }
            embassy_time::Timer::after_ticks(1).await;
        }
        self.stop_dma();
        write32(IRQ_EOIOH0, 1 << EKD_IRQ_LCD_LINE);
        Self::settle();
        Ok(())
    }
}
