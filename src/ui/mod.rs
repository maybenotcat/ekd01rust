use crate::soc::{
    display::{Display, DisplayError, HEIGHT, WIDTH},
    time::Timer,
};
use alloc::{
    alloc::{alloc_zeroed, handle_alloc_error},
    boxed::Box,
    rc::Rc,
};
use core::alloc::Layout;
use core::time::Duration;
use slint::{
    ComponentHandle, PhysicalSize,
    platform::{
        Platform, PlatformError, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType, Rgb565Pixel},
    },
};
mod bindings;
pub mod data;
pub mod model;
pub mod power_flow;
pub mod presentation;

slint::include_modules!();

struct SlintPlatform {
    window: Rc<MinimalSoftwareWindow>,
}
impl Platform for SlintPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
    fn duration_since_start(&self) -> Duration {
        Duration::from_millis(embassy_time::Instant::now().as_ticks() * 1000 / 128)
    }
}

/// The renderer owns the framebuffer until DMA completion; no second task may draw.
#[repr(C, align(32))]
struct Framebuffer {
    pixels: [Rgb565Pixel; WIDTH * HEIGHT],
}
impl Framebuffer {
    fn allocate() -> Box<Self> {
        // The LCD requires 32-byte alignment; Vec<Rgb565Pixel> only promises two.
        // Allocate directly on the heap to avoid a 150 KiB stack temporary.
        let layout = Layout::new::<Self>();
        let pointer = unsafe { alloc_zeroed(layout) }.cast::<Self>();
        if pointer.is_null() {
            handle_alloc_error(layout);
        }
        // In the pinned renderer Rgb565Pixel is a transparent u16; zero is black.
        unsafe { Box::from_raw(pointer) }
    }
}

pub struct UserInterface {
    pub component: Dashboard,
    window: Rc<MinimalSoftwareWindow>,
    framebuffer: Box<Framebuffer>,
    display: Display,
    statistics: Rc<slint::VecModel<Statistic>>,
    menu_items: Rc<slint::VecModel<MenuEntry>>,
    first_frame: bool,
}
impl UserInterface {
    pub fn new(display: Display) -> Result<Self, PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        slint::platform::set_platform(Box::new(SlintPlatform {
            window: window.clone(),
        }))
        .expect("install Slint platform once");
        window.set_size(PhysicalSize::new(WIDTH as u32, HEIGHT as u32));
        let component = Dashboard::new()?;
        component.set_build_id(crate::BUILD_ID.into());
        component.set_git_commit(env!("EKD01_GIT_COMMIT").into());
        component.set_git_modified(env!("EKD01_GIT_MODIFIED") == "true");
        let statistics = Rc::new(slint::VecModel::default());
        let menu_items = Rc::new(slint::VecModel::default());
        component.set_statistics(statistics.clone().into());
        component.set_menu_items(menu_items.clone().into());
        component.show()?;
        let framebuffer = Framebuffer::allocate();
        Ok(Self {
            component,
            window,
            framebuffer,
            display,
            statistics,
            menu_items,
            first_frame: true,
        })
    }
    pub async fn render(&mut self) -> Result<bool, DisplayError> {
        if self.first_frame {
            crate::soc::DebugUart::log("[SLINT] FIRST_TIMERS_ENTER\n");
        }
        slint::platform::update_timers_and_animations();
        if self.first_frame {
            crate::soc::DebugUart::log("[SLINT] FIRST_RASTER_ENTER\n");
        }
        let mut dirty = None;
        self.window.draw_if_needed(|renderer| {
            let region = renderer.render(&mut self.framebuffer.pixels, WIDTH);
            let origin = region.bounding_box_origin();
            let size = region.bounding_box_size();
            if size.width != 0 && size.height != 0 {
                dirty = Some([
                    origin.x as u16,
                    origin.y as u16,
                    (origin.x as u32 + size.width - 1) as u16,
                    (origin.y as u32 + size.height - 1) as u16,
                ]);
            }
        });
        if self.first_frame {
            crate::soc::DebugUart::log("[SLINT] FIRST_RASTER_EXIT\n");
        }
        if let Some(region) = dirty {
            // Rgb565Pixel is repr(transparent) over u16 in the pinned Slint version.
            let words = unsafe {
                core::slice::from_raw_parts(
                    self.framebuffer.pixels.as_ptr().cast::<u16>(),
                    self.framebuffer.pixels.len(),
                )
            };
            if self.first_frame {
                crate::soc::DebugUart::hex("[LCD] FIRST_FRAME_ADDRESS", words.as_ptr() as u32);
                crate::soc::DebugUart::log("[LCD] FIRST_PRESENT_ENTER\n");
            }
            self.display.present(words, region).await?;
            if self.first_frame {
                crate::soc::DebugUart::log("[LCD] FIRST_PRESENT_EXIT\n");
            }
            let first = self.first_frame;
            self.first_frame = false;
            return Ok(first);
        }
        Ok(false)
    }
    pub fn now() -> u32 {
        Timer::now()
    }
}
