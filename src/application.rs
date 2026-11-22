use crate::{
    bluetooth::{
        state::{BluetoothControl, BluetoothState},
        telemetry::TelemetryService,
    },
    motor::{
        client::{BasicStatus, MotorClient},
        settings::BlockId,
    },
    preferences::SettingsManager,
    soc::{
        DebugUart,
        backlight::Backlight,
        buttons::Buttons,
        display::Display,
        flash::Flash,
        pmu::Pmu,
        rtc::{ClockTime, Rtc},
        time::Timer,
    },
    storage::{FlashBus, SettingsStore},
    ui::{
        UserInterface,
        model::{Actions, UiModel},
        power_flow::{PowerFlow, PowerState},
        presentation::DeviceInfo,
    },
};
use alloc::rc::Rc;
use core::cell::RefCell;
use embassy_executor::Spawner;

pub struct Application {
    pmu: Pmu,
    rtc: Rtc,
    backlight: Backlight,
    buttons: Buttons,
    motor: MotorClient,
    settings: SettingsManager,
    model: UiModel,
    ui: UserInterface,
    power: PowerFlow,
    device: DeviceInfo,
    seen_travel: u32,
    seen_history: u32,
    next_render: u32,
    last_activity: u32,
    last_clock: u32,
    save_pending: bool,
    sequence: u16,
}
impl Application {
    pub async fn initialize(spawner: Spawner) -> Self {
        let flash = Flash::initialize();
        let jedec = flash.jedec;
        let available = flash.available();
        let bus: FlashBus = Rc::new(RefCell::new(flash));
        let mut settings = SettingsManager::new(SettingsStore::new(bus.clone()));
        let preferences = settings.load();
        let model = UiModel::new(preferences);
        let mut pmu = Pmu::initialize();
        let display = Display::initialize(&mut pmu)
            .await
            .expect("LCD initialization");
        let ui = UserInterface::new(display).expect("Slint platform");
        let mut backlight = Backlight::initialize();
        backlight.set_level(model.preferences.settings.brightness);
        let now = Timer::now();
        let buttons = Buttons::initialize(now);
        let mut motor = MotorClient::start(now);
        motor.controls(
            model.preferences.settings.levels,
            0,
            false,
            model.preferences.settings.power_mode,
        );
        BluetoothControl::set_enabled(model.preferences.settings.bluetooth);
        spawner.spawn(bluetooth_task(bus).unwrap());
        DebugUart::log("[UI] APPLICATION_READY\n");
        Self {
            pmu,
            rtc: Rtc,
            backlight,
            buttons,
            motor,
            settings,
            model,
            ui,
            power: PowerFlow::default(),
            device: DeviceInfo {
                jedec,
                flash_writable: available,
                bluetooth: BluetoothState::Off,
            },
            seen_travel: 0,
            seen_history: 0,
            next_render: now,
            last_activity: now,
            last_clock: now.wrapping_sub(128),
            save_pending: false,
            sequence: 0,
        }
    }
    fn shutdown(&mut self, now: u32, wait_release: bool) {
        BluetoothControl::set_enabled(false);
        self.power.request(now, wait_release);
        self.motor.stop();
        self.power_status("Powering off");
    }
    fn power_status(&self, text: &str) {
        self.ui.component.set_status(
            crate::ui::data::translate(text, self.model.preferences.settings.language != 0).into(),
        );
    }
    fn actions(&mut self, actions: Actions, now: u32) {
        let model = &mut self.model;
        if actions.contains(Actions::CONTROLS) {
            self.motor.controls(
                model.preferences.settings.levels,
                model.assist,
                model.lights,
                model.preferences.settings.power_mode,
            );
        }
        if actions.contains(Actions::BRIGHTNESS) {
            self.backlight
                .set_level(model.preferences.settings.brightness);
        }
        if actions.contains(Actions::BLUETOOTH) {
            BluetoothControl::set_enabled(model.preferences.settings.bluetooth);
        }
        if actions.contains(Actions::RESET_TRIP) {
            self.motor.clear_trip();
        }
        if actions.contains(Actions::CLOCK) {
            model.clock_valid = self.rtc.set(ClockTime {
                hour: model.hour,
                minute: model.minute,
            });
        }
        if actions.contains(Actions::ADV_READ) {
            self.motor.read_block(BlockId::AssistLimits, now);
        }
        if actions.contains(Actions::START_READ) {
            self.motor.read_block(BlockId::StartResponse, now);
        }
        if actions.contains(Actions::ADV_APPLY) {
            self.motor.apply_block(
                BlockId::AssistLimits,
                model.limits.block,
                model.limits.revision,
                now,
            );
        }
        if actions.contains(Actions::START_APPLY) {
            self.motor.apply_block(
                BlockId::StartResponse,
                model.start.block,
                model.start.revision,
                now,
            );
        }
        if actions.contains(Actions::MOTOR_APPLY) {
            self.motor.apply_basic(
                [
                    if model.basic.valid == 7 {
                        model.basic.limit_x10 * 10
                    } else {
                        0
                    },
                    model.basic.wheel,
                    model.basic.circumference,
                ],
                now,
            );
        }
        if actions.contains(Actions::SAVE) {
            self.save_pending = true;
        }
        if actions.contains(Actions::SHUTDOWN) {
            self.shutdown(now, true);
        }
    }
    fn synchronize(&mut self, now: u32) {
        let bike = self.motor.status();
        let telemetry = &bike.telemetry;
        self.model
            .preferences
            .record_distance(telemetry.travel_m.wrapping_sub(self.seen_travel));
        self.seen_travel = telemetry.travel_m;
        let count = telemetry
            .history_serial
            .wrapping_sub(self.seen_history)
            .min(10);
        self.seen_history = telemetry.history_serial;
        for index in (0..count as usize).rev() {
            self.model.preferences.record_error(telemetry.errors[index]);
        }
        self.model.advanced_readback(&bike.advanced);
        let basic = &mut self.model.basic;
        if bike.basic_state == BasicStatus::Verified
            && [basic.limit_x10 * 10, basic.wheel, basic.circumference] == bike.requested_basic
        {
            basic.dirty = false;
        }
        if telemetry.valid & 4 != 0 {
            self.model
                .motor_readback(telemetry.limit, telemetry.wheel, telemetry.circumference);
        }
        if now.wrapping_sub(self.last_clock) >= 128 && !self.model.navigation.editing {
            self.last_clock = now;
            if let Some(clock) = self.rtc.read() {
                self.model.hour = clock.hour;
                self.model.minute = clock.minute;
                self.model.clock_valid = true;
            } else {
                self.model.clock_valid = false;
            }
        }
        if telemetry.electric_fresh(now) && telemetry.speed != 0 {
            self.last_activity = now;
        }
        if self.power.state == PowerState::Running
            && self.model.preferences.settings.auto_off_minutes != 0
            && now.wrapping_sub(self.last_activity)
                >= self.model.preferences.settings.auto_off_minutes as u32 * 60 * 128
        {
            self.shutdown(now, false);
        }
    }
    fn save(&mut self) -> bool {
        let result = self.settings.save(&self.model.preferences);
        self.model.save_error = result.is_err();
        if let Err(error) = result {
            use core::fmt::Write;
            let _ = writeln!(DebugUart, "[SETTINGS] SAVE_FAILED: {error:?}");
        }
        !self.model.save_error
    }
    fn complete_shutdown(&mut self, now: u32) {
        if !self.power.links_ready(
            now,
            self.motor.status().stopped,
            BluetoothControl::stopped(),
        ) {
            return;
        }
        if !self.motor.status().stopped {
            self.motor.force_stop();
        }
        let saved = self.save();
        self.power.save_finished(saved);
        if !saved {
            self.power_status("Settings save failed");
            return;
        }
        if !self.rtc.release_power() {
            self.power.state = PowerState::ReleaseFailed;
            self.power_status("Power control error");
            return;
        }
        self.backlight.off();
        self.power.state = PowerState::Off;
    }
    pub async fn run(mut self) -> ! {
        let mut first_update = true;
        DebugUart::log("[UI] RUN_ENTER\n");
        let mut first_service = true;
        loop {
            let now = Timer::now();
            self.motor.service(now, |code| self.buttons.mcu(code, now));
            if first_service {
                DebugUart::log("[BIKE] FIRST_SERVICE_EXIT\n");
                first_service = false;
            }
            self.buttons.service(&self.pmu, now);
            let mut actions = Actions::empty();
            for _ in 0..16 {
                let keys = self.buttons.take(now);
                let transitions =
                    keys.pressed | keys.released | keys.short | keys.long | keys.repeat;
                if transitions != 0 || keys.held != 0 {
                    self.last_activity = now;
                }
                if self.power.retry(now, keys.short & 4 != 0) {
                    self.power_status("Powering off");
                }
                let action = if self.power.state == PowerState::Running {
                    self.model.step(keys, now)
                } else {
                    Actions::empty()
                };
                actions |= action;
                self.actions(action, now);
                if keys.released & 4 != 0 {
                    self.power.released(now);
                }
                if transitions == 0 {
                    break;
                }
            }
            self.synchronize(now);
            self.complete_shutdown(now);
            if self.power.state == PowerState::Running
                && (actions.contains(Actions::REDRAW)
                    || now.wrapping_sub(self.next_render) as i32 >= 0)
            {
                self.device.bluetooth = BluetoothControl::state();
                if first_update {
                    DebugUart::log("[UI] FIRST_UPDATE_ENTER\n");
                }
                self.ui
                    .update(&self.model, self.motor.status(), &self.device, now);
                if first_update {
                    DebugUart::log("[UI] FIRST_UPDATE_EXIT\n");
                    first_update = false;
                }
                self.next_render = now.wrapping_add(32);
                self.sequence = self.sequence.wrapping_add(1);
                TelemetryService::publish(self.motor.status(), now, self.sequence);
            }
            if self.power.state != PowerState::Off {
                self.ui.render().await.expect("LCD DMA");
            }
            // Publish the requested screen before a synchronous settings operation.
            if self.save_pending && self.power.state == PowerState::Running {
                self.save_pending = false;
                self.save();
                self.next_render = Timer::now();
            }
            embassy_time::Timer::after_ticks(1).await;
        }
    }
}

#[embassy_executor::task]
async fn bluetooth_task(bus: FlashBus) {
    crate::bluetooth::service::run(bus).await;
}
