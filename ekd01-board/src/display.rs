use crate::display_profile::EKD_LCD_DMA_TIMEOUT_TICKS;
use crate::{panels::*, pins, power::Power};
use core::marker::PhantomData;
use mt2502_hal::environment::Runtime;
use mt2502_hal::{gpio::Pin, lcd::Lcd, registers::*};

#[derive(Clone, Copy)]
pub struct DisplayPins {
    pub reset: u32,
    pub cs: u32,
    pub clock: u32,
    pub bus: [u32; 3],
}
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
pub struct Display<P: Runtime> {
    _environment: PhantomData<P>,
    reset: Pin,
    cs: Pin,
    clock: Pin,
    _bus: [Pin; 3],
    failed: bool,
    first_transfer: bool,
    landscape_mode: u8,
    flipped: bool,
}
impl<P: Runtime> Display<P> {
    pub async fn initialize(pmu: &mut Power<P>) -> Result<Self, DisplayError> {
        P::log("[LCD] INIT_ENTER\n");
        let pins = DisplayPins {
            reset: pins::LCD_RESET,
            cs: pins::LCD_CS,
            clock: pins::LCD_CLOCK,
            bus: pins::LCD_OTHER_BUS,
        };
        let mut display = unsafe {
            Self {
                _environment: PhantomData,

                reset: Pin::claim(pins.reset),
                cs: Pin::claim(pins.cs),
                clock: Pin::claim(pins.clock),
                _bus: pins.bus.map(|number| Pin::claim(number)),
                failed: false,
                first_transfer: true,
                landscape_mode: 0,
                flipped: false,
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
        Lcd::enable_clock();
        pmu.enable_lcd_supply();
        P::delay_ms(20).await;
        P::log("[LCD] SUPPLY_SETTLED; BUS_INIT_ENTER\n");
        Lcd::configure_bus(&crate::display_profile::BUS);
        P::log("[LCD] BUS_INIT_EXIT; READ_ID_ENTER\n");
        let id = display.read_id().await;
        P::hex("[LCD] ID", id);
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
        P::log(PANELS[profile].name);
        P::log("\n");
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
                4 => P::delay_ms(value as u64).await,
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
        display.landscape_mode = orientation;
        display.window(0, 0, 319, 239);
        P::hex("[LCD] LANDSCAPE_MADCTL", orientation as u32);
        P::log("[LCD] PANEL_INIT_EXIT\n");
        Ok(display)
    }
    /// Rotate the panel scan by 180 degrees without changing RGB order or DMA geometry.
    /// The caller must resend the complete framebuffer when this returns true.
    pub async fn set_flipped(&mut self, flipped: bool) -> Result<bool, DisplayError> {
        if self.failed {
            return Err(DisplayError::Disabled);
        }
        if self.flipped == flipped {
            return Ok(false);
        }
        let started = P::now();
        while Lcd::status() & (LCD_STATUS_RUN_BIT | LCD_STATUS_BUSY_BIT) != 0 {
            if P::now().wrapping_sub(started) >= EKD_LCD_DMA_TIMEOUT_TICKS {
                return self.fail(DisplayError::DmaIdle);
            }
            P::delay_ticks(1).await;
        }
        let mode = self.landscape_mode
            ^ if flipped {
                (DCS_MADCTL_MX | DCS_MADCTL_MY) as u8
            } else {
                0
            };
        self.command(DCS_SET_ADDRESS_MODE as u8);
        self.datum(mode);
        self.window(0, 0, (WIDTH - 1) as u16, (HEIGHT - 1) as u16);
        self.flipped = flipped;
        P::hex("[LCD] ROTATION_MADCTL", mode as u32);
        Ok(true)
    }
    fn settle() {
        Lcd::settle();
    }
    fn bus_write(&mut self, data: bool, value: u16, width: u8) {
        Lcd::write_serial(data, value, width);
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
        Lcd::repeat(value, count);
    }
    async fn read_id(&mut self) -> u32 {
        self.reset.set(true);
        P::delay_ms(200).await;
        self.reset.set(false);
        P::delay_ms(10).await;
        self.reset.set(true);
        P::delay_ms(120).await;
        self.cs.set(false);
        self.bus_write(false, DCS_READ_DISPLAY_ID as u16, 2);
        self.clock.mode(0);
        self.clock.output();
        self.clock.set(false);
        self.clock.set(true);
        self.clock.mode(1);
        let mut id = 0;
        for _ in 0..4 {
            id = (id << 8) | Lcd::read_byte() as u32;
        }
        self.cs.set(true);
        P::delay_ms(1).await;
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
        Lcd::stop_dma();
        self.cs.set(true);
    }
    fn fail<T>(&mut self, error: DisplayError) -> Result<T, DisplayError> {
        self.stop_dma();
        self.failed = true;
        P::hex("[LCD] DMA_STATUS", Lcd::status());
        Err(error)
    }
    pub async fn present(&mut self, pixels: &[u16], region: [u16; 4]) -> Result<(), DisplayError> {
        if self.failed {
            return Err(DisplayError::Disabled);
        }
        let [x0, y0, x1, y1] = region;
        let address = mt2502_hal::dma::physical_address(pixels.as_ptr());
        let trace = core::mem::replace(&mut self.first_transfer, false);
        if trace {
            let cpsr = cpu_status();
            P::hex("[LCD] DMA_ENTER_CPSR", cpsr);
            P::hex("[LCD] DMA_ENTER_STATUS", Lcd::status());
            P::hex("[LCD] DMA_REGION_BEGIN", (y0 as u32) << 16 | x0 as u32);
            P::hex("[LCD] DMA_REGION_END", (y1 as u32) << 16 | x1 as u32);
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
            P::hex("[LCD] FRAMEBUFFER_ADDRESS", address);
            P::hex("[LCD] FRAMEBUFFER_PIXELS", pixels.len() as u32);
            P::hex("[LCD] REGION_BEGIN", (y0 as u32) << 16 | x0 as u32);
            P::hex("[LCD] REGION_END", (y1 as u32) << 16 | x1 as u32);
            return self.fail(DisplayError::Framebuffer);
        }
        let started = P::now();
        if trace {
            P::hex("[LCD] DMA_IDLE_WAIT_TICK", started);
        }
        while Lcd::status() & (LCD_STATUS_RUN_BIT | LCD_STATUS_BUSY_BIT) != 0 {
            if P::now().wrapping_sub(started) >= EKD_LCD_DMA_TIMEOUT_TICKS {
                return self.fail(DisplayError::DmaIdle);
            }
            P::delay_ticks(1).await;
        }
        if trace {
            P::log("[LCD] DMA_IDLE_READY\n");
        }
        Lcd::prepare_dma();
        self.window(x0, y0, x1, y1);
        if trace {
            P::log("[LCD] DMA_WINDOW_READY\n");
        }
        Lcd::configure_rgb565(
            address,
            WIDTH as u32,
            HEIGHT as u32,
            region,
            EKD_LCD_DESTINATION_BIAS,
        );
        if trace {
            P::log("[LCD] DMA_REGISTERS_READY\n");
        }
        unsafe {
            Lcd::flush_framebuffer(pixels.as_ptr(), WIDTH * HEIGHT);
        }
        self.cs.set(false);
        Lcd::clear_pending();
        let started = P::now();
        Lcd::start();
        if trace {
            P::hex("[LCD] DMA_STARTED_STATUS", Lcd::status());
            P::hex("[LCD] DMA_STARTED_TICK", P::now());
        }
        let mut saw_run = false;
        let mut complete = false;
        loop {
            let running = Lcd::status() & (LCD_STATUS_RUN_BIT | LCD_STATUS_BUSY_BIT) != 0;
            complete |= Lcd::completion_pending();
            saw_run |= running;
            if (complete || saw_run) && !running {
                break;
            }
            if P::now().wrapping_sub(started) >= EKD_LCD_DMA_TIMEOUT_TICKS {
                return self.fail(DisplayError::DmaTimeout);
            }
            P::delay_ticks(1).await;
        }
        self.stop_dma();
        Lcd::acknowledge_interrupt();
        Self::settle();
        Ok(())
    }
}

fn cpu_status() -> u32 {
    #[cfg(target_arch = "arm")]
    {
        mt2502_hal::cpu::status()
    }
    #[cfg(not(target_arch = "arm"))]
    {
        0
    } // Diagnostic unavailable on the native test host.
}
