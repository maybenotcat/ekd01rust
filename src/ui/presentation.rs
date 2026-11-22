use super::{
    Row, UserInterface,
    data::{Item, translate},
    model::{UiModel, View},
};
use crate::{
    bluetooth::state::BluetoothState,
    motor::{
        client::{BasicStatus, BikeStatus},
        settings::Parameter,
        transfer::{RequestResult, TransferStatus},
    },
};
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
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
impl Presenter<'_> {
    fn tr<'a>(&self, key: &'a str) -> &'a str {
        translate(key, self.model.preferences.settings.language != 0)
    }
    fn unit(&self) -> &str {
        self.tr(if self.model.preferences.settings.imperial {
            "mi"
        } else {
            "km"
        })
    }
    fn distance(&self, metric: u32) -> u32 {
        if self.model.preferences.settings.imperial {
            (metric as u64 * 621371 / 1000000) as u32
        } else {
            metric
        }
    }
    fn trip(&self) -> String {
        let value = self.distance(self.bike.telemetry.trip_m);
        format!("{}.{:02} {}", value / 1000, value % 1000 / 10, self.unit())
    }
    fn voltage(&self) -> String {
        let value = if self.bike.telemetry.electric_fresh(self.now) {
            self.bike.telemetry.voltage as u32
        } else if self.bike.voltage_fresh(self.now) {
            self.bike.mcu_voltage_mv / 10
        } else {
            return "--.- V".into();
        };
        format!("{}.{} V", value / 100, value / 10 % 10)
    }
    fn battery(&self) -> String {
        let t = &self.bike.telemetry;
        if t.ride_fresh(self.now) && t.battery <= 100 {
            format!("{}%", t.battery)
        } else {
            "--%".into()
        }
    }
    fn row(&self, label: &str, value: String, active: bool) -> Row {
        Row {
            label: self.tr(label).into(),
            value: value.into(),
            active,
        }
    }
    fn result(result: RequestResult) -> &'static str {
        match result {
            RequestResult::Accepted => "Apply",
            RequestResult::Busy => "Busy",
            RequestResult::Offline => "CAN offline",
            RequestResult::NeedsNeutral => "Set assist 0",
            RequestResult::Invalid => "Set values",
        }
    }
    fn basic_status(&self) -> String {
        let key = if self.bike.basic_result != RequestResult::Accepted {
            Self::result(self.bike.basic_result)
        } else {
            match self.bike.basic_state {
                BasicStatus::Idle => "Apply",
                BasicStatus::Queued => "Queued",
                BasicStatus::Sent => "Sent",
                BasicStatus::Verified => "Readback OK",
                BasicStatus::Timeout => "No reply",
                BasicStatus::BadValues => "Set values",
                BasicStatus::Offline => "CAN offline",
                BasicStatus::Busy => "Busy",
                BasicStatus::Neutral => "Set assist 0",
            }
        };
        self.tr(key).into()
    }
    fn advanced_status(&self, item: Item) -> String {
        use crate::motor::settings::BlockId;
        let start = matches!(item, Item::StartRead | Item::StartApply);
        let id = if start {
            BlockId::StartResponse
        } else {
            BlockId::AssistLimits
        };
        let draft = if start {
            &self.model.start
        } else {
            &self.model.limits
        };
        let transfer = &self.bike.advanced;
        let key = if transfer.requested_block == Some(id)
            && transfer.request_result != RequestResult::Accepted
        {
            Self::result(transfer.request_result)
        } else if matches!(item, Item::AdvRead | Item::StartRead) && draft.dirty {
            "Draft pending"
        } else if item == Item::AdvApply && draft.dirty && transfer.state == TransferStatus::Ready {
            "Modified"
        } else if transfer.block_id != Some(id) {
            if draft.valid { "Loaded" } else { "Read first" }
        } else {
            match transfer.state {
                TransferStatus::Idle => "Read first",
                TransferStatus::Reading => "Reading",
                TransferStatus::Ready => "Loaded",
                TransferStatus::Writing => "Writing",
                TransferStatus::Verifying => "Verifying",
                TransferStatus::Verified => "Verified",
                TransferStatus::Timeout => "No reply",
                TransferStatus::BadBlock => "Bad block",
                TransferStatus::Mismatch => "Mismatch",
                TransferStatus::Offline => "CAN offline",
                TransferStatus::Neutral => "Set assist 0",
                TransferStatus::Busy => "Busy",
            }
        };
        self.tr(key).into()
    }
    fn menu_value(&self, item: Item) -> String {
        use Item::*;
        let s = &self.model.preferences.settings;
        let d = &self.model.basic;
        let tr = |key| self.tr(key).to_string();
        match item {
            Brightness => format!(
                "{}%",
                [10, 30, 50, 75, 100][s.brightness.saturating_sub(1).min(4) as usize]
            ),
            Units => self.unit().into(),
            Bluetooth => tr(if s.bluetooth { "On" } else { "Off" }),
            AutoOff => {
                if s.auto_off_minutes == 0 {
                    tr("Off")
                } else {
                    format!(
                        "{} {}",
                        s.auto_off_minutes,
                        if s.language != 0 { "мин" } else { "min" }
                    )
                }
            }
            Hour | Minute => {
                if self.model.clock_valid {
                    format!("{:02}:{:02}", self.model.hour, self.model.minute)
                } else {
                    "--:--".into()
                }
            }
            Language => if s.language != 0 {
                "Русский"
            } else {
                "English"
            }
            .into(),
            Levels => format!("{}", s.levels),
            Limit => {
                if d.valid & 1 != 0 {
                    format!(
                        "{}.{} {}",
                        d.limit_x10 / 10,
                        d.limit_x10 % 10,
                        self.tr("km/h")
                    )
                } else {
                    format!("-- {}", self.tr("km/h"))
                }
            }
            Wheel => {
                if d.valid & 2 != 0 {
                    format!("{}.{} in", d.wheel / 10, d.wheel % 10)
                } else {
                    "-- in".into()
                }
            }
            Circumference => {
                if d.valid & 4 != 0 {
                    format!(
                        "{} {}",
                        d.circumference,
                        if s.language != 0 { "мм" } else { "mm" }
                    )
                } else {
                    "-- mm".into()
                }
            }
            Apply => self.basic_status(),
            AdvRead | AdvApply | StartRead | StartApply => self.advanced_status(item),
            Start | Current | Speed => format!(
                "{} {}",
                s.levels,
                if s.language != 0 {
                    "уровней"
                } else {
                    "levels"
                }
            ),
            AdvDiscard | StartDiscard => tr(
                if (if item == StartDiscard {
                    &self.model.start
                } else {
                    &self.model.limits
                })
                .dirty
                {
                    "Modified"
                } else {
                    ">"
                },
            ),
            PowerMode => format!(
                "{} {}",
                if s.language != 0 {
                    "Режим"
                } else {
                    "Mode"
                },
                s.power_mode + 1
            ),
            ServiceInterval => {
                if s.service_interval_km == 0 {
                    tr("Off")
                } else {
                    format!("{} {}", s.service_interval_km, self.tr("km"))
                }
            }
            ServiceDistance => format!(
                "{} {}",
                self.model.preferences.since_service_m / 1000,
                self.tr("km")
            ),
            Odometer => format!(
                "{} {}",
                self.model.preferences.odometer_m / 1000,
                self.tr("km")
            ),
            Trip | FactoryReset | ServiceReset | ErrorClear => {
                if self.model.navigation.editing
                    && self.model.item(self.model.navigation.selected) == item
                {
                    tr(if self.model.navigation.confirm_reset {
                        "Yes"
                    } else {
                        "No"
                    })
                } else if item == Trip {
                    self.trip()
                } else if item == ServiceReset {
                    format!(
                        "{} {}",
                        self.model.preferences.since_service_m / 1000,
                        self.tr("km")
                    )
                } else {
                    ">".into()
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
                        return "--".into();
                    }
                    let value = draft
                        .block
                        .get(parameter, s.levels, index as u8 + 1)
                        .unwrap_or(0);
                    if parameter == Parameter::Start {
                        format!("{value} / 8")
                    } else {
                        format!("{value}%")
                    }
                } else {
                    ">".into()
                }
            }
        }
    }
    fn menu_rows(&self) -> Vec<Row> {
        let first = self.model.navigation.selected / 5 * 5;
        (first..(first + 5).min(self.model.count()))
            .map(|index| {
                let item = self.model.item(index);
                self.row(
                    item.label(),
                    self.menu_value(item),
                    index == self.model.navigation.selected,
                )
            })
            .collect()
    }
    fn ride_rows(&self) -> Vec<Row> {
        let t = &self.bike.telemetry;
        let range = self.distance(t.range as u32);
        let ride = t.ride_fresh(self.now);
        let electric = t.electric_fresh(self.now);
        let range = if ride {
            format!("{}.{} {}", range / 100, range / 10 % 10, self.unit())
        } else {
            "--".into()
        };
        let rpm = if self.model.preferences.settings.language != 0 {
            "об/мин"
        } else {
            "rpm"
        };
        let cadence = if ride {
            format!("{} {rpm}", t.cadence)
        } else {
            format!("-- {rpm}")
        };
        let temperature = |value| {
            if electric {
                format!("{value} C")
            } else {
                "-- C".into()
            }
        };
        [
            ("TRIP", self.trip()),
            ("RANGE", range),
            ("CADENCE", cadence),
            ("BATTERY", self.battery()),
            ("MOTOR", temperature(t.motor_c)),
            ("CONTROLLER", temperature(t.controller_c)),
        ]
        .into_iter()
        .map(|(label, value)| self.row(label, value, false))
        .collect()
    }
    fn information(&self) -> String {
        use Item::*;
        let t = &self.bike.telemetry;
        let ru = self.model.preferences.settings.language != 0;
        match self.model.navigation.info_page.unwrap_or(BatteryInfo) {
            BatteryInfo => format!(
                "{}  {}\nSOC  {}",
                if ru {
                    "Напряжение"
                } else {
                    "Total voltage"
                },
                self.voltage(),
                self.battery()
            ),
            CtrlInfo => {
                if t.valid & 4 != 0 {
                    format!(
                        "{}  {}.{:02} {}\n{}  {}.{} in\n{}  {} mm\nCAN RX  {}\n{}  --",
                        self.tr("Speed Limit"),
                        t.limit / 100,
                        t.limit % 100,
                        self.tr("km/h"),
                        self.tr("Wheel Size"),
                        t.wheel / 10,
                        t.wheel % 10,
                        self.tr("Wheel Length"),
                        t.circumference,
                        self.bike.rx,
                        if ru {
                            "Серийный номер / версия"
                        } else {
                            "Serial / version"
                        }
                    )
                } else {
                    if ru {
                        "Ожидание данных контроллера"
                    } else {
                        "Waiting for controller data"
                    }
                    .into()
                }
            }
            DisplayInfo => format!(
                "LCD  320x240\nFLASH  {:06X} / 8 MiB\nPSRAM  8 MiB\nMCU  {:02X?}\n{:02}:{:02}",
                self.device.jedec, self.bike.mcu_version, self.model.hour, self.model.minute
            ),
            TorqueInfo => if ru {
                "Нет данных датчика момента\nСерийный номер / версия  --"
            } else {
                "No torque sensor data\nSerial / version  --"
            }
            .into(),
            Errors => {
                let count = self
                    .model
                    .preferences
                    .errors
                    .iter()
                    .take_while(|value| **value != 0)
                    .count();
                if count == 0 {
                    return self.tr("No recorded errors").into();
                }
                let index = self.model.navigation.error_page % count;
                let error = self.model.preferences.errors[index];
                format!(
                    "{} {:02X}\n{} {} / cmd {:02X}:{}\n{} / {}",
                    if ru { "Ошибка" } else { "Error" },
                    error & 255,
                    if ru { "Источник" } else { "Source" },
                    error >> 24,
                    (error >> 8) & 255,
                    (error >> 16) & 255,
                    index + 1,
                    count
                )
            }
            _ => self.tr("Unavailable").into(),
        }
    }
}

impl UserInterface {
    fn replace_rows(&self, rows: Vec<Row>) {
        if self.rows.row_count() != rows.len() {
            self.rows.set_vec(rows);
            return;
        }
        for (index, row) in rows.into_iter().enumerate() {
            if self.rows.row_data(index).as_ref() != Some(&row) {
                self.rows.set_row_data(index, row);
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
        let t = &bike.telemetry;
        let c = &self.component;
        c.set_view(model.navigation.view as i32);
        c.set_status_error(model.save_error);
        c.set_status(if model.navigation.view == View::Dashboard {
            "".into()
        } else if model.save_error {
            p.tr("Save error").into()
        } else if model.navigation.editing {
            p.tr("EDIT").into()
        } else if model.navigation.view == View::Menu {
            format!("{} / {}", model.navigation.selected + 1, model.count()).into()
        } else {
            "".into()
        });
        match model.navigation.view {
            View::Dashboard => {
                c.set_bluetooth(device.bluetooth as i32);
                c.set_lights(model.lights);
                c.set_lights_text(p.tr("LIGHT").into());
                c.set_assist_caption(p.tr("ASSIST").into());
                c.set_assist_text(
                    format!("{} / {}", model.assist, model.preferences.settings.levels).into(),
                );
                c.set_voltage(p.voltage().into());
                c.set_trip_caption(p.tr("TRIP").into());
                c.set_trip_text(p.trip().into());
                c.set_battery_text(p.battery().into());
                c.set_battery(if t.ride_fresh(now) && t.battery <= 100 {
                    t.battery as i32
                } else {
                    0
                });
                c.set_speed_unit(
                    p.tr(if model.preferences.settings.imperial {
                        "mph"
                    } else {
                        "km/h"
                    })
                    .into(),
                );
                c.set_fault(t.error != 0);
                let error = if t.error != 0 {
                    format!(
                        "{} {:02X}",
                        if model.preferences.settings.language != 0 {
                            "ОШИБКА"
                        } else {
                            "ERROR"
                        },
                        t.error & 255
                    )
                } else if model.preferences.settings.service_interval_km != 0
                    && model.preferences.since_service_m / 1000
                        >= model.preferences.settings.service_interval_km
                {
                    p.tr("SERVICE DUE").into()
                } else {
                    String::new()
                };
                c.set_error(error.into());
                if t.electric_fresh(now) {
                    c.set_speed_available(true);
                    let speed = p.distance(t.speed as u32);
                    let power = t.voltage as i64 * t.current as i64 / 10000;
                    c.set_compact_speed(speed >= 10000);
                    c.set_speed_integer(format!("{}", speed / 100).into());
                    c.set_speed_fraction(format!(".{}", speed / 10 % 10).into());
                    c.set_speed_digit_count(if speed >= 10000 {
                        3
                    } else if speed >= 1000 {
                        2
                    } else {
                        1
                    });
                    c.set_power(
                        format!(
                            "{power} {}",
                            if model.preferences.settings.language != 0 {
                                "Вт"
                            } else {
                                "W"
                            }
                        )
                        .into(),
                    );
                    c.set_speed_fill(
                        (speed
                            / (if model.preferences.settings.imperial {
                                40
                            } else {
                                60
                            }))
                        .min(100) as i32,
                    );
                    c.set_power_fill((power.max(0) / 20).min(100) as i32);
                } else {
                    c.set_speed_available(false);
                    c.set_speed_integer("--".into());
                    c.set_speed_fraction(".-".into());
                    c.set_speed_digit_count(2);
                    c.set_compact_speed(false);
                    c.set_power(
                        if model.preferences.settings.language != 0 {
                            "-- Вт"
                        } else {
                            "-- W"
                        }
                        .into(),
                    );
                    c.set_speed_fill(0);
                    c.set_power_fill(0);
                }
            }
            View::Ride => {
                c.set_page_title(p.tr("RIDE STATISTICS").into());
                self.replace_rows(p.ride_rows());
            }
            View::Menu => {
                c.set_page_title(p.tr(model.navigation.page.title()).into());
                self.replace_rows(p.menu_rows());
            }
            View::Information => {
                c.set_page_title(
                    p.tr(model
                        .navigation
                        .info_page
                        .unwrap_or(Item::BatteryInfo)
                        .label())
                        .into(),
                );
                c.set_info_body(p.information().into());
            }
            View::Diagnostics => {
                c.set_page_title(p.tr("Diagnostics").into());
                c.set_info_body(format!("PSRAM  OK\nFLASH  {} / {:06X}\nCAN RX  {}  TX  {}\nMCU RX  {}\nADC  {} mV / {}\nUART errors  {}\nBad frames  {}\nLast CAN  {:08X}",
                    if device.flash_writable { "OK" } else { "READ" },device.jedec,bike.rx,bike.tx,bike.mcu_rx,bike.mcu_voltage_mv,
                    if bike.voltage_fresh(now) { "fresh" } else { "stale" },bike.uart_errors,bike.malformed,bike.last_id).into());
            }
        }
    }
}
