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
mod text;
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
    let mut data = dashboard.get_dashboard_data();
    data.classic_speed_fill = 50;
    dashboard.set_dashboard_data(data);
    for theme in 0..=1 {
        dashboard.set_theme(theme);
        for (index, speed) in ["--.-", "0.0", "25.3", "100.0"].iter().enumerate() {
            let mut data = dashboard.get_dashboard_data();
            data.speed_available = *speed != "--.-";
            data.speed_tenths = if data.speed_available {
                (speed.parse::<f32>().unwrap() * 10.0) as i32
            } else {
                0
            };
            dashboard.set_dashboard_data(data);
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
    let mut data = dashboard.get_dashboard_data();
    data.fault = true;
    data.error_code = 0x21;
    dashboard.set_dashboard_data(data);
    for scene in ["error", "menu"] {
        if scene == "menu" {
            dashboard.set_view(Screen::Menu);
            dashboard.set_menu_page(MenuPage::Display);
            dashboard.set_menu_selected(2);
            dashboard.set_menu_items(
                std::rc::Rc::new(slint::VecModel::from(vec![
                    MenuEntry {
                        kind: MenuItem::Units,
                        value: DisplayValue {
                            kind: ValueKind::DistanceUnit,
                            ..Default::default()
                        },
                    },
                    MenuEntry {
                        kind: MenuItem::Brightness,
                        value: sample_value(ValueKind::Percent, 50, 0),
                    },
                    MenuEntry {
                        kind: MenuItem::Theme,
                        value: DisplayValue {
                            kind: ValueKind::Theme,
                            flag: true,
                            ..Default::default()
                        },
                    },
                    MenuEntry {
                        kind: MenuItem::AutoOff,
                        value: sample_value(ValueKind::Minutes, 5, 0),
                    },
                    MenuEntry {
                        kind: MenuItem::Trip,
                        value: DisplayValue::default(),
                    },
                ]))
                .into(),
            );
        }
        assert!(window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 320);
        }));
        let bytes =
            unsafe { std::slice::from_raw_parts(pixels.as_ptr().cast::<u8>(), pixels.len() * 2) };
        let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../build/ekd01rust-theme-{scene}.rgb565"));
        std::fs::write(output, bytes).unwrap();
    }
    check_settings_paging(&dashboard, &window, &mut pixels);
    text::check(&dashboard);
}

fn check_settings_paging(
    dashboard: &Dashboard,
    window: &MinimalSoftwareWindow,
    pixels: &mut [Rgb565Pixel],
) {
    use slint::Model;
    let text = dashboard.global::<MenuText>();
    assert_eq!(text.invoke_label(MenuItem::Brightness, false), "Brightness");
    assert_eq!(text.invoke_label(MenuItem::Brightness, true), "Яркость");
    assert_eq!(text.invoke_label(MenuItem::Level9, true), "Уровень 9");
    assert_eq!(
        text.invoke_title(MenuPage::Advanced, false),
        "ADVANCED MOTOR"
    );
    assert_eq!(
        text.invoke_title(MenuPage::Advanced, true),
        "ПАРАМЕТРЫ МОТОРА"
    );

    // Adding an offscreen second page must not change the current frame.
    let first_page = pixels.to_vec();
    let mut entries: Vec<_> = dashboard.get_menu_items().iter().collect();
    entries.extend([
        MenuEntry {
            kind: MenuItem::Hour,
            value: sample_value(ValueKind::Clock, 12, 34),
        },
        MenuEntry {
            kind: MenuItem::Minute,
            value: sample_value(ValueKind::Clock, 12, 34),
        },
        MenuEntry {
            kind: MenuItem::Language,
            value: DisplayValue {
                kind: ValueKind::Language,
                flag: false,
                ..Default::default()
            },
        },
        MenuEntry {
            kind: MenuItem::Service,
            value: DisplayValue {
                kind: ValueKind::Navigation,
                ..Default::default()
            },
        },
        MenuEntry {
            kind: MenuItem::Back,
            value: DisplayValue {
                kind: ValueKind::Navigation,
                ..Default::default()
            },
        },
    ]);
    let entries = Rc::new(slint::VecModel::from(entries));
    dashboard.set_menu_items(entries.clone().into());
    window.draw_if_needed(|renderer| {
        renderer.render(pixels, 320);
    });
    assert!(pixels == first_page);

    // Selection is absolute across the full model, not relative to five rows.
    dashboard.set_menu_selected(7);
    assert!(window.draw_if_needed(|renderer| {
        renderer.render(pixels, 320);
    }));
    let second_page = pixels.to_vec();
    assert!(second_page != first_page);
    assert_eq!(pixels[110 * 320 + 10], first_page[110 * 320 + 10]);

    // Updating the same model must repaint a value without changing selection.
    entries.set_row_data(
        7,
        MenuEntry {
            kind: MenuItem::Language,
            value: DisplayValue {
                kind: ValueKind::Language,
                flag: true,
                ..Default::default()
            },
        },
    );
    assert!(window.draw_if_needed(|renderer| {
        renderer.render(pixels, 320);
    }));
    assert!(pixels != second_page);
    let updated = pixels.to_vec();
    dashboard.set_russian(true);
    assert!(window.draw_if_needed(|renderer| {
        renderer.render(pixels, 320);
    }));
    assert!(pixels != updated);

    // Returning to page one restores every pixel, including the selection.
    dashboard.set_russian(false);
    dashboard.set_menu_selected(2);
    assert!(window.draw_if_needed(|renderer| {
        renderer.render(pixels, 320);
    }));
    assert!(pixels == first_page);
    println!("Settings: paging, selection, in-place updates and language switching OK");
}

fn sample_value(kind: ValueKind, whole: i32, fraction: i32) -> DisplayValue {
    DisplayValue {
        kind,
        whole,
        fraction,
        available: true,
        flag: false,
    }
}
