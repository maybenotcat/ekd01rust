use super::{
    CounterValue, DashboardData, DiagnosticsData, DisplayValue, InformationData, MenuEntry,
    Statistic, StatisticKind, StatusCode, UserInterface, ValueKind,
    data::Item,
    model::{Page, UiModel, View},
};
use crate::{
    bluetooth::state::BluetoothState,
    motor::{
        client::{BasicStatus, BikeStatus},
        settings::{BlockId, Parameter},
        transfer::{RequestResult, TransferStatus},
    },
};
use alloc::vec::Vec;
use slint::Model;

pub struct DeviceInfo {
    pub jedec: u32,
    pub flash_writable: bool,
    pub bluetooth: BluetoothState,
}
struct Presenter<'a> {
    model: &'a UiModel,
    bike: &'a BikeStatus,
    device: &'a DeviceInfo,
    now: u32,
}
impl DisplayValue {
    fn number(kind: ValueKind, whole: i32) -> Self {
        Self {
            kind,
            whole,
            available: true,
            ..Self::default()
        }
    }
    fn decimal(kind: ValueKind, whole: i32, fraction: i32) -> Self {
        Self {
            fraction,
            ..Self::number(kind, whole)
        }
    }
    fn missing(kind: ValueKind) -> Self {
        Self {
            kind,
            ..Self::default()
        }
    }
    fn option(kind: ValueKind, flag: bool) -> Self {
        Self {
            kind,
            flag,
            ..Self::default()
        }
    }
}
impl From<u32> for CounterValue {
    fn from(value: u32) -> Self {
        Self {
            upper: (value / 10_000) as i32,
            lower: (value % 10_000) as i32,
        }
    }
}
impl Presenter<'_> {
    fn distance(&self, metric: u32) -> u32 {
        if self.model.preferences.settings.imperial {
            (metric as u64 * 621371 / 1000000) as u32
        } else {
            metric
        }
    }
    fn trip(&self) -> DisplayValue {
        let value = self.distance(self.bike.telemetry.trip_m);
        DisplayValue::decimal(
            ValueKind::DistanceHundredths,
            (value / 1000) as i32,
            (value % 1000 / 10) as i32,
        )
    }
    fn voltage(&self) -> DisplayValue {
        let value = if self.bike.telemetry.electric_fresh(self.now) {
            self.bike.telemetry.voltage as u32
        } else if self.bike.voltage_fresh(self.now) {
            self.bike.mcu_voltage_mv / 10
        } else {
            return DisplayValue::missing(ValueKind::Voltage);
        };
        DisplayValue::decimal(
            ValueKind::Voltage,
            (value / 100) as i32,
            (value / 10 % 10) as i32,
        )
    }
    fn battery(&self) -> DisplayValue {
        let t = &self.bike.telemetry;
        if t.ride_fresh(self.now) && t.battery <= 100 {
            DisplayValue::number(ValueKind::Percent, t.battery as i32)
        } else {
            DisplayValue::missing(ValueKind::Percent)
        }
    }
    fn result(result: RequestResult) -> StatusCode {
        match result {
            RequestResult::Accepted => StatusCode::Apply,
            RequestResult::Busy => StatusCode::Busy,
            RequestResult::Offline => StatusCode::Offline,
            RequestResult::NeedsNeutral => StatusCode::Neutral,
            RequestResult::Invalid => StatusCode::Invalid,
        }
    }
    fn basic_status(&self) -> StatusCode {
        if self.bike.basic_result != RequestResult::Accepted {
            return Self::result(self.bike.basic_result);
        }
        match self.bike.basic_state {
            BasicStatus::Idle => StatusCode::Apply,
            BasicStatus::Reading => StatusCode::Reading,
            BasicStatus::Loaded => StatusCode::Loaded,
            BasicStatus::Queued => StatusCode::Queued,
            BasicStatus::Sent => StatusCode::Sent,
            BasicStatus::Verified => StatusCode::ReadbackOk,
            BasicStatus::Timeout => StatusCode::Timeout,
            BasicStatus::BadValues => StatusCode::Invalid,
            BasicStatus::Offline => StatusCode::Offline,
            BasicStatus::Busy => StatusCode::Busy,
            BasicStatus::Neutral => StatusCode::Neutral,
        }
    }
    fn advanced_status(&self, id: BlockId) -> StatusCode {
        let draft = if id == BlockId::StartResponse {
            &self.model.start
        } else {
            &self.model.limits
        };
        let transfer = &self.bike.advanced;
        if transfer.requested_block == Some(id)
            && transfer.request_result != RequestResult::Accepted
        {
            return Self::result(transfer.request_result);
        }
        if draft.dirty && transfer.state == TransferStatus::Ready {
            return StatusCode::Modified;
        }
        if transfer.block_id != Some(id) {
            return if draft.valid {
                StatusCode::Loaded
            } else {
                StatusCode::ReadFirst
            };
        }
        match transfer.state {
            TransferStatus::Idle => StatusCode::ReadFirst,
            TransferStatus::Reading => StatusCode::Reading,
            TransferStatus::Ready => StatusCode::Loaded,
            TransferStatus::Writing => StatusCode::Writing,
            TransferStatus::Verifying => StatusCode::Verifying,
            TransferStatus::Verified => StatusCode::Verified,
            TransferStatus::Timeout => StatusCode::Timeout,
            TransferStatus::BadBlock => StatusCode::BadBlock,
            TransferStatus::Mismatch => StatusCode::Mismatch,
            TransferStatus::Offline => StatusCode::Offline,
            TransferStatus::Neutral => StatusCode::Neutral,
            TransferStatus::Busy => StatusCode::Busy,
        }
    }
    fn motor_page_status(&self) -> StatusCode {
        let status = match self.model.navigation.page {
            Page::Motor => self.basic_status(),
            Page::Advanced | Page::Current | Page::Speed => {
                self.advanced_status(BlockId::AssistLimits)
            }
            Page::Start => self.advanced_status(BlockId::StartResponse),
            _ => return StatusCode::None,
        };
        if self.model.motor_busy && !self.bike.active {
            StatusCode::Offline
        } else if !self.model.motor_busy && self.model.assist != 0 {
            StatusCode::Neutral
        } else {
            status
        }
    }
    fn menu_value(&self, item: Item) -> DisplayValue {
        use Item::*;
        let s = &self.model.preferences.settings;
        let d = &self.model.basic;
        match item {
            Theme => DisplayValue::option(
                ValueKind::Theme,
                s.theme == crate::preferences::DashboardTheme::Classic,
            ),
            Brightness => DisplayValue::number(
                ValueKind::Percent,
                [10, 30, 50, 75, 100][s.brightness.saturating_sub(1).min(4) as usize],
            ),
            Units => DisplayValue::missing(ValueKind::DistanceUnit),
            Bluetooth => DisplayValue::option(ValueKind::Toggle, s.bluetooth),
            AutoOff => DisplayValue::number(ValueKind::Minutes, s.auto_off_minutes as i32),
            Hour | Minute => DisplayValue {
                available: self.model.clock_valid,
                ..DisplayValue::decimal(
                    ValueKind::Clock,
                    self.model.hour as i32,
                    self.model.minute as i32,
                )
            },
            Language => DisplayValue::option(ValueKind::Language, s.language != 0),
            Levels => DisplayValue::number(ValueKind::Number, s.levels as i32),
            Limit => DisplayValue {
                available: d.valid & 1 != 0,
                ..DisplayValue::decimal(
                    ValueKind::MetricSpeed,
                    (d.limit_x10 / 10) as i32,
                    (d.limit_x10 % 10) as i32,
                )
            },
            Wheel => DisplayValue {
                available: d.valid & 2 != 0,
                ..DisplayValue::decimal(
                    ValueKind::Wheel,
                    (d.wheel / 10) as i32,
                    (d.wheel % 10) as i32,
                )
            },
            Circumference => DisplayValue {
                available: d.valid & 4 != 0,
                ..DisplayValue::number(ValueKind::Millimetres, d.circumference as i32)
            },
            Start | Current | Speed => DisplayValue::number(ValueKind::Levels, s.levels as i32),
            PowerMode => DisplayValue::number(ValueKind::Mode, s.power_mode as i32 + 1),
            ServiceInterval => {
                if s.service_interval_km == 0 {
                    DisplayValue::option(ValueKind::Toggle, false)
                } else {
                    DisplayValue::number(ValueKind::Kilometres, s.service_interval_km as i32)
                }
            }
            ServiceDistance | Odometer => DisplayValue::number(
                ValueKind::Kilometres,
                (if item == Odometer {
                    self.model.preferences.odometer_m
                } else {
                    self.model.preferences.since_service_m
                } / 1000) as i32,
            ),
            Trip | FactoryReset | ServiceReset | ErrorClear => {
                if self.model.navigation.editing
                    && self.model.item(self.model.navigation.selected) == item
                {
                    DisplayValue::option(
                        ValueKind::Confirmation,
                        self.model.navigation.confirm_reset,
                    )
                } else if item == Trip {
                    self.trip()
                } else if item == ServiceReset {
                    DisplayValue::number(
                        ValueKind::Kilometres,
                        (self.model.preferences.since_service_m / 1000) as i32,
                    )
                } else {
                    DisplayValue::missing(ValueKind::Navigation)
                }
            }
            _ => {
                let levels = [
                    Level1, Level2, Level3, Level4, Level5, Level6, Level7, Level8, Level9,
                ];
                if let (Some(index), Some(parameter)) = (
                    levels.iter().position(|value| *value == item),
                    self.model.navigation.page.parameter(),
                ) {
                    let draft = if parameter == Parameter::Start {
                        &self.model.start
                    } else {
                        &self.model.limits
                    };
                    if !draft.valid {
                        return DisplayValue::missing(ValueKind::Unavailable);
                    }
                    let value = draft
                        .block
                        .get(parameter, s.levels, index as u8 + 1)
                        .unwrap_or(0);
                    DisplayValue::number(
                        if parameter == Parameter::Start {
                            ValueKind::StartResponse
                        } else {
                            ValueKind::Percent
                        },
                        value as i32,
                    )
                } else {
                    DisplayValue::missing(ValueKind::Navigation)
                }
            }
        }
    }
    fn menu_entries(&self) -> Vec<MenuEntry> {
        (0..self.model.count())
            .map(|index| {
                let item = self.model.item(index);
                MenuEntry {
                    kind: item.into(),
                    value: self.menu_value(item),
                }
            })
            .collect()
    }
    fn ride_rows(&self) -> Vec<Statistic> {
        let t = &self.bike.telemetry;
        let range = self.distance(t.range as u32);
        let ride = t.ride_fresh(self.now);
        let electric = t.electric_fresh(self.now);
        [
            (StatisticKind::Trip, self.trip()),
            (
                StatisticKind::Range,
                if ride {
                    DisplayValue::decimal(
                        ValueKind::DistanceTenths,
                        (range / 100) as i32,
                        (range / 10 % 10) as i32,
                    )
                } else {
                    DisplayValue::missing(ValueKind::Unavailable)
                },
            ),
            (
                StatisticKind::Cadence,
                DisplayValue {
                    available: ride,
                    ..DisplayValue::number(ValueKind::Cadence, t.cadence as i32)
                },
            ),
            (StatisticKind::Battery, self.battery()),
            (
                StatisticKind::Motor,
                DisplayValue {
                    available: electric,
                    ..DisplayValue::number(ValueKind::Temperature, t.motor_c as i32)
                },
            ),
            (
                StatisticKind::Controller,
                DisplayValue {
                    available: electric,
                    ..DisplayValue::number(ValueKind::Temperature, t.controller_c as i32)
                },
            ),
        ]
        .into_iter()
        .map(|(kind, value)| Statistic { kind, value })
        .collect()
    }
    fn dashboard(&self) -> DashboardData {
        let t = &self.bike.telemetry;
        let odometer = self.distance(self.model.preferences.odometer_m);
        let s = &self.model.preferences.settings;
        DashboardData {
            speed_tenths: (self.distance(t.speed as u32) / 10) as i32,
            speed_available: t.electric_fresh(self.now),
            classic_speed_fill: if t.electric_fresh(self.now) {
                (t.speed as u32 / 75).min(100) as i32
            } else {
                0
            },
            power: DisplayValue {
                available: t.electric_fresh(self.now),
                ..DisplayValue::number(
                    ValueKind::Power,
                    (t.voltage as i64 * t.current as i64 / 10000) as i32,
                )
            },
            voltage: self.voltage(),
            battery: self.battery(),
            trip: self.trip(),
            odometer: DisplayValue::decimal(
                ValueKind::DistanceTenths,
                (odometer / 1000) as i32,
                (odometer % 1000 / 100) as i32,
            ),
            assist: self.model.assist as i32,
            lights: self.model.lights,
            bluetooth: self.device.bluetooth as i32,
            fault: t.error != 0,
            error_code: (t.error & 255) as i32,
            service_due: s.service_interval_km != 0
                && self.model.preferences.since_service_m / 1000 >= s.service_interval_km,
        }
    }
    fn information(&self) -> InformationData {
        let t = &self.bike.telemetry;
        let errors = &self.model.preferences.errors;
        let count = errors.iter().take_while(|value| **value != 0).count();
        let index = if count == 0 {
            0
        } else {
            self.model.navigation.error_page % count
        };
        let error = if count == 0 { 0 } else { errors[index] };
        let version = self.bike.mcu_version;
        InformationData {
            voltage: self.voltage(),
            battery: self.battery(),
            controller_available: t.valid & 4 != 0,
            limit: t.limit as i32,
            wheel: t.wheel as i32,
            circumference: t.circumference as i32,
            can_rx: self.bike.rx.into(),
            flash_jedec: self.device.jedec as i32,
            mcu0: version[0] as i32,
            mcu1: version[1] as i32,
            mcu2: version[2] as i32,
            mcu3: version[3] as i32,
            mcu4: version[4] as i32,
            hour: self.model.hour as i32,
            minute: self.model.minute as i32,
            error_count: count as i32,
            error_position: index as i32 + 1,
            error_code: (error & 255) as i32,
            error_source: (error >> 24) as i32,
            error_command: ((error >> 8) & 255) as i32,
            error_subcommand: ((error >> 16) & 255) as i32,
        }
    }
    fn diagnostics(&self) -> DiagnosticsData {
        DiagnosticsData {
            flash_writable: self.device.flash_writable,
            flash_jedec: self.device.jedec as i32,
            can_rx: self.bike.rx.into(),
            can_tx: self.bike.tx.into(),
            mcu_rx: self.bike.mcu_rx.into(),
            adc_mv: self.bike.mcu_voltage_mv.into(),
            adc_fresh: self.bike.voltage_fresh(self.now),
            uart_errors: self.bike.uart_errors.into(),
            bad_frames: self.bike.malformed.into(),
            last_can_high: (self.bike.last_id >> 16) as i32,
            last_can_low: (self.bike.last_id & 0xffff) as i32,
        }
    }
}

impl UserInterface {
    fn update_model<T: Clone + PartialEq + 'static>(model: &slint::VecModel<T>, values: Vec<T>) {
        if model.row_count() != values.len() {
            model.set_vec(values);
            return;
        }
        for (index, value) in values.into_iter().enumerate() {
            if model.row_data(index).as_ref() != Some(&value) {
                model.set_row_data(index, value);
            }
        }
    }
    pub fn update(&self, model: &UiModel, bike: &BikeStatus, device: &DeviceInfo, now: u32) {
        let p = Presenter {
            model,
            bike,
            device,
            now,
        };
        let c = &self.component;
        c.set_view(model.navigation.view.into());
        c.set_russian(model.preferences.settings.language != 0);
        c.set_imperial(model.preferences.settings.imperial);
        c.set_theme(model.preferences.settings.theme as i32);
        c.set_status_error(model.save_error);
        c.set_status_code(if model.navigation.view == View::Dashboard {
            StatusCode::None
        } else if model.save_error {
            StatusCode::SaveError
        } else if model.navigation.editing {
            StatusCode::Edit
        } else if model.navigation.view == View::Menu {
            match p.motor_page_status() {
                StatusCode::None => StatusCode::PagePosition,
                code => code,
            }
        } else {
            StatusCode::None
        });
        match model.navigation.view {
            View::Dashboard => c.set_dashboard_data(p.dashboard()),
            View::Ride => Self::update_model(&self.statistics, p.ride_rows()),
            View::Menu => {
                c.set_menu_page(model.navigation.page.into());
                c.set_menu_selected(model.navigation.selected as i32);
                Self::update_model(&self.menu_items, p.menu_entries());
            }
            View::Information => {
                c.set_information_item(
                    model
                        .navigation
                        .info_page
                        .unwrap_or(Item::BatteryInfo)
                        .into(),
                );
                c.set_information_data(p.information());
            }
            View::Diagnostics => c.set_diagnostics_data(p.diagnostics()),
        }
    }
}
