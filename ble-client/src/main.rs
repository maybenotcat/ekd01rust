//! Read-only desktop BLE client. The firmware target remains the workspace default.
mod backend;
mod presentation;
mod protocol;

use backend::{Command, DeviceInfo, Event, Worker};
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::{
    cell::RefCell,
    path::Path,
    rc::Rc,
    sync::mpsc,
    time::{Duration, Instant},
};
slint::include_modules!();

fn cards(snapshot: Option<&protocol::Snapshot>) -> Vec<Metric> {
    let values = snapshot
        .map(presentation::values)
        .unwrap_or_else(|| vec!["—".into(); presentation::LABELS.len()]);
    presentation::LABELS
        .iter()
        .zip(values)
        .map(|(label, value)| Metric {
            label: (*label).into(),
            value: value.into(),
        })
        .collect()
}
struct ClientView {
    devices: Vec<DeviceInfo>,
    generation: u64,
    last_received: Option<Instant>,
    metrics: Rc<VecModel<Metric>>,
}
impl ClientView {
    fn clear(&mut self) {
        self.metrics.set_vec(cards(None));
        self.last_received = None;
    }
    fn apply(
        &mut self,
        window: &TelemetryWindow,
        event: Event,
        commands: &tokio::sync::mpsc::UnboundedSender<Command>,
    ) {
        match event {
            Event::Devices(devices) => {
                let selected = usize::try_from(window.get_selected())
                    .ok()
                    .and_then(|index| self.devices.get(index))
                    .map(|device| device.id.clone());
                let labels: Vec<SharedString> = devices
                    .iter()
                    .map(|device| device.label.clone().into())
                    .collect();
                window.set_devices(ModelRc::new(VecModel::from(labels)));
                window.set_selected(
                    selected
                        .and_then(|id| devices.iter().position(|device| device.id == id))
                        .or_else(|| (!devices.is_empty()).then_some(0))
                        .map(|index| index as i32)
                        .unwrap_or(-1),
                );
                self.devices = devices;
            }
            Event::State {
                status,
                scanning,
                connected,
                busy,
                generation,
            } => {
                self.generation = generation;
                window.set_status(status.into());
                window.set_scanning(scanning);
                window.set_connected(connected);
                window.set_busy(busy);
                if !connected {
                    self.clear();
                }
            }
            Event::Snapshot { value, generation }
                if generation == self.generation && window.get_connected() =>
            {
                self.metrics.set_vec(cards(Some(&value)));
                window.set_status(presentation::status(&value).into());
                self.last_received = Some(Instant::now());
            }
            Event::NotificationEnded(generation)
                if generation == self.generation && window.get_connected() =>
            {
                self.clear();
                window.set_busy(true);
                window.set_status("Telemetry stream closed".into());
                let _ = commands.send(Command::Disconnect);
            }
            _ => {}
        }
    }
    fn expire(&mut self, window: &TelemetryWindow) {
        if self
            .last_received
            .is_some_and(|received| received.elapsed() > protocol::STALE_AFTER)
        {
            self.clear();
            window.set_status("No fresh telemetry".into());
        }
    }
}

fn run_window() -> Result<(), Box<dyn std::error::Error>> {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .select()?;
    let window = TelemetryWindow::new()?;
    let metrics = Rc::new(VecModel::from(cards(None)));
    window.set_metrics(metrics.clone().into());
    let view = Rc::new(RefCell::new(ClientView {
        devices: Vec::new(),
        generation: 0,
        last_received: None,
        metrics,
    }));
    let (events, receiver) = mpsc::channel();
    let worker = Worker::start(events);
    let commands = worker.commands.clone();
    let weak = window.as_weak();
    window.on_scan(move || {
        if let Some(window) = weak.upgrade() {
            window.set_busy(true);
            if commands.send(Command::Scan).is_err() {
                window.set_busy(false);
                window.set_status("BLE worker unavailable".into());
            }
        }
    });
    let commands = worker.commands.clone();
    let weak = window.as_weak();
    let selected_view = view.clone();
    window.on_connect(move |selected| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let command = if window.get_connected() {
            Some(Command::Disconnect)
        } else {
            usize::try_from(selected).ok().and_then(|index| {
                selected_view
                    .borrow()
                    .devices
                    .get(index)
                    .map(|device| Command::Connect(device.id.clone()))
            })
        };
        if let Some(command) = command {
            window.set_busy(true);
            if commands.send(command).is_err() {
                window.set_busy(false);
                window.set_status("BLE worker unavailable".into());
            }
        }
    });
    let weak = window.as_weak();
    let commands = worker.commands.clone();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(100), move || {
        if let Some(window) = weak.upgrade() {
            let mut view = view.borrow_mut();
            for event in receiver.try_iter() {
                view.apply(&window, event, &commands);
            }
            view.expire(&window);
        }
    });
    window.run()?;
    drop(worker);
    Ok(())
}

fn preview(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use slint::{
        PhysicalSize,
        platform::{
            Platform, PlatformError, WindowAdapter,
            software_renderer::{MinimalSoftwareWindow, RepaintBufferType, Rgb565Pixel},
        },
    };
    struct Offscreen(Rc<MinimalSoftwareWindow>);
    impl Platform for Offscreen {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(self.0.clone())
        }
        fn duration_since_start(&self) -> Duration {
            Duration::ZERO
        }
    }
    let adapter = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(Offscreen(adapter.clone())))?;
    adapter.set_size(PhysicalSize::new(940, 720));
    let window = TelemetryWindow::new()?;
    let snapshot = protocol::Snapshot::decode(include_bytes!("../fixtures/snapshot-v1.bin"))
        .ok_or("Invalid preview fixture")?;
    window.set_metrics(ModelRc::new(VecModel::from(cards(Some(&snapshot)))));
    window.set_devices(ModelRc::new(VecModel::from(vec![SharedString::from(
        "EKD01Free   F2:26:EC:54:F4:E6   -55 dBm",
    )])));
    window.set_selected(0);
    window.set_connected(true);
    window.set_status(presentation::status(&snapshot).into());
    window.show()?;
    slint::platform::update_timers_and_animations();
    let mut pixels = vec![Rgb565Pixel::default(); 940 * 720];
    if !adapter.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 940);
    }) {
        return Err("No preview frame rendered".into());
    }
    let mut ppm = b"P6\n940 720\n255\n".to_vec();
    for pixel in pixels {
        let word = pixel.0;
        ppm.extend([
            ((word >> 11) * 255 / 31) as u8,
            (((word >> 5) & 63) * 255 / 63) as u8,
            ((word & 31) * 255 / 31) as u8,
        ]);
    }
    std::fs::write(path, ppm)?;
    println!("Rendered {} without Bluetooth access", path.display());
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [] => run_window(),
        [command] if command == "--self-test" => {
            protocol::self_test()?;
            presentation::self_test()?;
            println!(
                "PASS: C/C# fixture, page ordering, signed values, stale data, sequence wrap and ADC fallback"
            );
            Ok(())
        }
        [command, path] if command == "--preview" => preview(Path::new(path)),
        [command] if command == "--help" || command == "-h" => {
            println!(
                "EKD01 BLE telemetry example\nRun without arguments for the window.\n--self-test: protocol checks without Bluetooth\n--preview FILE.ppm: render the window without Bluetooth"
            );
            Ok(())
        }
        _ => Err("Unknown arguments; use --help".into()),
    }
}
