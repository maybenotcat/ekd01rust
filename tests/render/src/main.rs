//! Offscreen native rendering of the actual Slint UI; no firmware/CPU emulation.
use slint::{
    ComponentHandle, PhysicalSize,
    platform::{
        Platform, PlatformError, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType, Rgb565Pixel},
    },
};
use std::{
    rc::Rc,
    time::{Duration, Instant},
};
slint::include_modules!();
struct Offscreen {
    window: Rc<MinimalSoftwareWindow>,
}
impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
    fn duration_since_start(&self) -> Duration {
        Duration::ZERO
    }
}
fn main() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(Offscreen {
        window: window.clone(),
    }))
    .unwrap();
    window.set_size(PhysicalSize::new(320, 240));
    let dashboard = Dashboard::new().unwrap();
    dashboard.show().unwrap();
    let mut pixels = vec![Rgb565Pixel::default(); 320 * 240];
    dashboard.set_power("742 W".into());
    dashboard.set_battery(78);
    dashboard.set_battery_text("78%".into());
    dashboard.set_trip_text("12.34 km".into());
    dashboard.set_odometer_text("321.0 km".into());
    dashboard.set_assist_level(5);
    dashboard.set_voltage("52.7 V".into());
    dashboard.set_bluetooth(3);
    dashboard.set_lights(true);
    dashboard.set_classic_speed_fill(50);
    dashboard.set_power_fill(37);
    for theme in 0..=1 {
        dashboard.set_theme(theme);
        for (index, speed) in ["--.-", "0.0", "25.3", "100.0"].iter().enumerate() {
            dashboard.set_speed_available(*speed != "--.-");
            dashboard.set_speed_tenths(if *speed == "--.-" {
                0
            } else {
                (speed.parse::<f32>().unwrap() * 10.0) as i32
            });
            let (integer, fraction) = speed.split_once('.').unwrap();
            dashboard.set_speed_integer(integer.into());
            dashboard.set_speed_fraction(format!(".{fraction}").into());
            dashboard.set_speed_digit_count(integer.len() as i32);
            dashboard.set_compact_speed(integer.len() >= 3);
            let start = Instant::now();
            slint::platform::update_timers_and_animations();
            let drawn = window.draw_if_needed(|renderer| {
                let region = renderer.render(&mut pixels, 320);
                if index == 0 && theme == 0 {
                    assert_eq!(region.bounding_box_size(), PhysicalSize::new(320, 240));
                }
            });
            assert!(drawn);
            let bytes = unsafe {
                std::slice::from_raw_parts(pixels.as_ptr().cast::<u8>(), pixels.len() * 2)
            };
            let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "../../build/ekd01rust-theme-{theme}-speed-{}.rgb565",
                speed.replace('.', "_")
            ));
            std::fs::write(output, bytes).unwrap();
            assert!(
                pixels.windows(2).any(|p| p[0] != p[1]),
                "Frame contains no visible UI"
            );
            println!("Slint speed {speed} rendered in {:?}", start.elapsed());
        }
    }
    dashboard.set_theme(1);
    dashboard.set_fault(true);
    dashboard.set_error("ERROR 21".into());
    for scene in ["error", "menu"] {
        if scene == "menu" {
            dashboard.set_view(2);
            dashboard.set_page_title("DISPLAY SETTING".into());
            dashboard.set_rows(std::rc::Rc::new(slint::VecModel::from(vec![
                Row { label: "Units".into(), value: "km".into(), active: false },
                Row { label: "Brightness".into(), value: "50%".into(), active: false },
                Row { label: "Main screen".into(), value: "Classic".into(), active: true },
                Row { label: "Auto Power Off".into(), value: "5 min".into(), active: false },
                Row { label: "Trip Reset".into(), value: "".into(), active: false },
            ])).into());
        }
        assert!(window.draw_if_needed(|renderer| { renderer.render(&mut pixels, 320); }));
        let bytes = unsafe { std::slice::from_raw_parts(pixels.as_ptr().cast::<u8>(), pixels.len() * 2) };
        let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../build/ekd01rust-theme-{scene}.rgb565"));
        std::fs::write(output, bytes).unwrap();
    }
}
