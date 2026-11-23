use super::data::Item;
use crate::{
    input::ButtonEvents,
    motor::{
        settings::{BlockId, Draft, Parameter},
        transfer::{AdvancedView, TransferStatus},
    },
    preferences::Preferences,
    protocol::wheel_circumference,
};

bitflags::bitflags! {
    #[derive(Clone,Copy,Default)]
    pub struct Actions:u16 {
        const REDRAW=1; const CONTROLS=2; const BRIGHTNESS=4; const RESET_TRIP=8;
        const SHUTDOWN=16; const MOTOR_APPLY=32; const CLOCK=64; const RESET_SETTINGS=128;
        const BLUETOOTH=256; const ADV_READ=512; const ADV_APPLY=1024;
        const START_READ=2048; const START_APPLY=4096; const SAVE=8192;
        const MOTOR_READ=16384;
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Dashboard,
    Ride,
    Menu,
    Diagnostics,
    Information,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Root,
    Display,
    Motor,
    Information,
    Advanced,
    Current,
    Speed,
    Start,
    Service,
}
const LEVEL_ITEMS: [Item; 9] = [
    Item::Level1,
    Item::Level2,
    Item::Level3,
    Item::Level4,
    Item::Level5,
    Item::Level6,
    Item::Level7,
    Item::Level8,
    Item::Level9,
];
impl Page {
    pub fn parameter(self) -> Option<Parameter> {
        match self {
            Self::Current => Some(Parameter::Current),
            Self::Speed => Some(Parameter::Speed),
            Self::Start => Some(Parameter::Start),
            _ => None,
        }
    }
    fn items(self) -> &'static [Item] {
        use Item::*;
        match self {
            Self::Root => &[Display, Motor, Information, Bluetooth, FactoryReset, Exit],
            Self::Display => &[
                Units, Brightness, Theme, AutoOff, Trip, Hour, Minute, Language, Service, Back,
            ],
            Self::Motor => &[Levels, Limit, Wheel, Circumference, Advanced, Back],
            Self::Information => &[BatteryInfo, Errors, ErrorClear, Diagnostics, Back],
            Self::Advanced => &[Current, Speed, Start, PowerMode, Back],
            Self::Service => &[
                ServiceInterval,
                ServiceDistance,
                Odometer,
                ServiceReset,
                Back,
            ],
            _ => &[],
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Root => "SETTINGS",
            Self::Display => "DISPLAY SETTING",
            Self::Motor => "MOTOR SETTINGS",
            Self::Information => "INFORMATION",
            Self::Advanced => "ADVANCED MOTOR",
            Self::Current => "CURRENT PER LEVEL",
            Self::Speed => "SPEED PER LEVEL",
            Self::Start => "START RESPONSE",
            Self::Service => "SERVICE",
        }
    }
    fn parent(self) -> (Self, Item) {
        match self {
            Self::Root => (Self::Root, Item::Exit),
            Self::Display => (Self::Root, Item::Display),
            Self::Motor => (Self::Root, Item::Motor),
            Self::Information => (Self::Root, Item::Information),
            Self::Advanced => (Self::Motor, Item::Advanced),
            Self::Current => (Self::Advanced, Item::Current),
            Self::Speed => (Self::Advanced, Item::Speed),
            Self::Start => (Self::Advanced, Item::Start),
            Self::Service => (Self::Display, Item::Service),
        }
    }
}
#[derive(Default)]
pub struct Navigation {
    pub view: View,
    pub page: Page,
    pub selected: usize,
    pub editing: bool,
    pub confirm_reset: bool,
    pub info_page: Option<Item>,
    pub error_page: usize,
    combo_started: Option<u32>,
    combo_fired: bool,
    repeated: u8,
}
#[derive(Default)]
pub struct BasicDraft {
    pub limit_x10: u16,
    pub wheel: u16,
    pub circumference: u16,
    pub valid: u8,
    pub dirty: bool,
}
pub struct UiModel {
    pub preferences: Preferences,
    pub navigation: Navigation,
    pub basic: BasicDraft,
    pub limits: Draft,
    pub start: Draft,
    pub assist: u8,
    pub lights: bool,
    pub hour: u8,
    pub minute: u8,
    pub clock_valid: bool,
    pub save_error: bool,
    pub motor_busy: bool,
    pub motor_online: bool,
}
impl UiModel {
    pub fn new(preferences: Preferences) -> Self {
        Self {
            preferences,
            navigation: Navigation::default(),
            basic: BasicDraft::default(),
            limits: Draft::default(),
            start: Draft::default(),
            assist: 0,
            lights: false,
            hour: 0,
            minute: 0,
            clock_valid: false,
            save_error: false,
            motor_busy: false,
            motor_online: false,
        }
    }
    pub fn count(&self) -> usize {
        if self.navigation.page.parameter().is_some() {
            self.preferences.settings.levels as usize + 1
        } else {
            self.navigation.page.items().len()
        }
    }
    pub fn item(&self, row: usize) -> Item {
        if self.navigation.page.parameter().is_some() {
            if row < self.preferences.settings.levels as usize {
                LEVEL_ITEMS[row]
            } else {
                Item::Back
            }
        } else {
            self.navigation
                .page
                .items()
                .get(row)
                .copied()
                .unwrap_or(Item::Back)
        }
    }
    pub fn enter(&mut self, page: Page) {
        self.navigation.page = page;
        self.navigation.selected = 0;
        self.navigation.editing = false;
        self.navigation.confirm_reset = false;
    }
    fn back(&mut self) {
        let (page, item) = self.navigation.page.parent();
        self.enter(page);
        self.navigation.selected = (0..self.count())
            .find(|row| self.item(*row) == item)
            .unwrap_or(0);
    }
    pub fn motor_readback(&mut self, limit: u16, wheel: u16, circumference: u16) {
        // A complete controller status must remain visible even when its values
        // lie outside our editor bounds. Validate writes at the protocol boundary.
        if self.navigation.editing || self.basic.dirty {
            return;
        }
        self.basic = BasicDraft {
            limit_x10: limit / 10,
            wheel,
            circumference,
            valid: 7,
            dirty: false,
        };
    }
    pub fn advanced_readback(&mut self, view: &AdvancedView) {
        self.start.synchronize(
            &view.start,
            BlockId::StartResponse,
            self.navigation.editing,
            view.state == TransferStatus::Verified && view.block_id == Some(BlockId::StartResponse),
        );
        self.limits.synchronize(
            &view.limits,
            BlockId::AssistLimits,
            self.navigation.editing,
            view.state == TransferStatus::Verified && view.block_id == Some(BlockId::AssistLimits),
        );
    }
    fn adjust(value: u32, direction: i32, min: u32, max: u32, step: u32) -> u32 {
        if value < min || value > max {
            return if direction > 0 { min } else { max };
        }
        if direction > 0 {
            value.saturating_add(step).min(max)
        } else {
            value.saturating_sub(step).max(min)
        }
    }
    fn draft_mut(&mut self) -> &mut Draft {
        if self.navigation.page == Page::Start {
            &mut self.start
        } else {
            &mut self.limits
        }
    }
    fn edit(&mut self, item: Item, direction: i32) -> Actions {
        use Item::*;
        let settings = &mut self.preferences.settings;
        let mut actions = Actions::REDRAW;
        match item {
            Brightness => {
                settings.brightness =
                    Self::adjust(settings.brightness as u32, direction, 1, 5, 1) as u8;
                actions |= Actions::BRIGHTNESS;
            }
            Units => settings.imperial = !settings.imperial,
            Bluetooth => {
                settings.bluetooth = !settings.bluetooth;
                actions |= Actions::BLUETOOTH;
            }
            Language => settings.language ^= 1,
            Theme => settings.theme.toggle(),
            PowerMode => {
                settings.power_mode =
                    (settings.power_mode as i32 + 4 + direction).rem_euclid(4) as u8;
                actions |= Actions::CONTROLS;
            }
            ServiceInterval => {
                settings.service_interval_km =
                    Self::adjust(settings.service_interval_km, direction, 0, 10000, 500)
            }
            AutoOff => {
                settings.auto_off_minutes =
                    Self::adjust(settings.auto_off_minutes as u32, direction, 0, 10, 1) as u8
            }
            Hour => {
                self.hour = (self.hour as i32 + 24 + direction).rem_euclid(24) as u8;
                self.clock_valid = true;
            }
            Minute => {
                self.minute = (self.minute as i32 + 60 + direction).rem_euclid(60) as u8;
                self.clock_valid = true;
            }
            Levels => {
                settings.levels = match (settings.levels, direction > 0) {
                    (3, true) | (9, false) => 5,
                    (5, true) | (3, false) => 9,
                    _ => 3,
                };
                self.assist = 0;
                actions |= Actions::CONTROLS;
            }
            Limit => {
                use crate::protocol::basic_bounds::*;
                self.basic.limit_x10 = Self::adjust(
                    self.basic.limit_x10 as u32,
                    direction,
                    (LIMIT_MIN_X100 / 10) as u32,
                    (LIMIT_MAX_X100 / 10) as u32,
                    10,
                ) as u16;
                self.basic.valid |= 1;
                self.basic.dirty = true;
            }
            Wheel => {
                use crate::protocol::basic_bounds::*;
                self.basic.wheel = Self::adjust(
                    self.basic.wheel as u32,
                    direction,
                    WHEEL_MIN_X10 as u32,
                    WHEEL_MAX_X10 as u32,
                    5,
                ) as u16;
                self.basic.circumference = wheel_circumference(self.basic.wheel).unwrap_or(0);
                self.basic.valid |= 6;
                self.basic.dirty = true;
            }
            Circumference => {
                use crate::protocol::basic_bounds::*;
                self.basic.circumference = Self::adjust(
                    self.basic.circumference as u32,
                    direction,
                    CIRCUMFERENCE_MIN_MM as u32,
                    CIRCUMFERENCE_MAX_MM as u32,
                    1,
                ) as u16;
                self.basic.valid |= 4;
                self.basic.dirty = true;
            }
            FactoryReset | Trip | ServiceReset | ErrorClear => {
                self.navigation.confirm_reset = !self.navigation.confirm_reset
            }
            _ => {
                if let Some(index) = LEVEL_ITEMS.iter().position(|candidate| *candidate == item) {
                    let parameter = self.navigation.page.parameter();
                    let levels = settings.levels;
                    if let Some(parameter) = parameter {
                        let draft = self.draft_mut();
                        if draft.valid {
                            let old = draft
                                .block
                                .get(parameter, levels, index as u8 + 1)
                                .unwrap_or(0);
                            let value = Self::adjust(
                                old as u32,
                                direction,
                                1,
                                parameter.maximum() as u32,
                                1,
                            ) as u8;
                            if draft.block.set(parameter, levels, index as u8 + 1, value) {
                                draft.dirty = true;
                            }
                        }
                    }
                }
            }
        }
        actions
    }
    fn finish_edit(&mut self, item: Item) -> Actions {
        use Item::*;
        self.navigation.editing = false;
        let mut actions = Actions::REDRAW;
        if matches!(item, Limit | Wheel | Circumference) && self.basic.dirty {
            actions |= Actions::MOTOR_APPLY;
        }
        if LEVEL_ITEMS.contains(&item) && self.draft_mut().dirty {
            actions |= if self.navigation.page == Page::Start {
                Actions::START_APPLY
            } else {
                Actions::ADV_APPLY
            };
        }
        if matches!(
            item,
            Brightness
                | Units
                | Bluetooth
                | Language
                | Theme
                | PowerMode
                | ServiceInterval
                | AutoOff
                | Levels
        ) {
            actions |= Actions::SAVE;
        }
        if matches!(item, Hour | Minute) {
            actions |= Actions::CLOCK;
        }
        if self.navigation.confirm_reset {
            match item {
                Trip => actions |= Actions::RESET_TRIP,
                ServiceReset => {
                    self.preferences.since_service_m = 0;
                    actions |= Actions::SAVE;
                }
                ErrorClear => {
                    self.preferences.errors.fill(0);
                    actions |= Actions::SAVE;
                }
                FactoryReset => {
                    let mut preferences = self.preferences;
                    preferences.factory_reset();
                    *self = Self::new(preferences);
                    self.navigation.view = View::Menu;
                    actions |= Actions::RESET_SETTINGS
                        | Actions::BRIGHTNESS
                        | Actions::CONTROLS
                        | Actions::BLUETOOTH
                        | Actions::SAVE;
                }
                _ => {}
            }
        }
        self.navigation.confirm_reset = false;
        actions
    }
    fn activate(&mut self, item: Item) -> Actions {
        if self.navigation.editing {
            return self.finish_edit(item);
        }
        use Item::*;
        let mut actions = Actions::REDRAW;
        match item {
            Display => self.enter(Page::Display),
            Motor => {
                self.enter(Page::Motor);
                if !self.basic.dirty {
                    self.basic.valid = 0;
                    actions |= Actions::MOTOR_READ;
                }
            }
            Information => self.enter(Page::Information),
            Advanced => {
                self.enter(Page::Advanced);
                if !self.limits.dirty {
                    self.limits.valid = false;
                    actions |= Actions::ADV_READ;
                }
            }
            Current | Speed => {
                self.enter(if item == Current {
                    Page::Current
                } else {
                    Page::Speed
                });
                if !self.limits.valid && !self.motor_busy {
                    actions |= Actions::ADV_READ;
                }
            }
            Start => {
                self.enter(Page::Start);
                if !self.start.dirty {
                    self.start.valid = false;
                    actions |= Actions::START_READ;
                }
            }
            Service => self.enter(Page::Service),
            Back => self.back(),
            Exit => {
                self.navigation.view = View::Dashboard;
                actions |= Actions::SAVE;
            }
            ServiceDistance | Odometer => {}
            Diagnostics => self.navigation.view = View::Diagnostics,
            BatteryInfo | CtrlInfo | DisplayInfo | TorqueInfo | Errors | Sensitivity => {
                self.navigation.info_page = Some(item);
                self.navigation.view = View::Information;
            }
            _ => {
                let motor_value =
                    matches!(item, Limit | Wheel | Circumference) || LEVEL_ITEMS.contains(&item);
                let loaded = if matches!(item, Limit | Wheel | Circumference) {
                    self.basic.valid == 7
                } else {
                    self.draft_mut().valid
                };
                if !motor_value
                    || loaded && !self.motor_busy && self.motor_online && self.assist == 0
                {
                    self.navigation.editing = true;
                }
            }
        }
        actions
    }
    pub fn step(&mut self, mut keys: ButtonEvents, now: u32) -> Actions {
        keys.short &= !self.navigation.repeated;
        self.navigation.repeated &= !keys.released;
        if keys.long & 4 != 0 {
            return Actions::SHUTDOWN | Actions::REDRAW;
        }
        let pair = keys.held & 3;
        if pair == 3 {
            let started = *self.navigation.combo_started.get_or_insert(now);
            if !self.navigation.combo_fired && now.wrapping_sub(started) >= 256 {
                let edit_actions = if self.navigation.editing {
                    self.finish_edit(self.item(self.navigation.selected))
                } else {
                    Actions::empty()
                };
                self.navigation.combo_fired = true;
                self.navigation.view = if self.navigation.view == View::Menu {
                    View::Dashboard
                } else {
                    View::Menu
                };
                self.enter(Page::Root);
                return Actions::REDRAW
                    | edit_actions
                    | if self.navigation.view == View::Dashboard {
                        Actions::SAVE
                    } else {
                        Actions::empty()
                    };
            }
            return Actions::empty();
        }
        if self.navigation.combo_started.is_some() {
            if pair == 0 {
                self.navigation.combo_started = None;
                self.navigation.combo_fired = false;
            }
            return Actions::empty();
        }
        if matches!(self.navigation.view, View::Information | View::Diagnostics) {
            if self.navigation.info_page == Some(Item::Errors)
                && self.navigation.view == View::Information
            {
                self.navigation.repeated |= (keys.pressed | keys.repeat) & 3;
                let count = self
                    .preferences
                    .errors
                    .iter()
                    .take_while(|value| **value != 0)
                    .count();
                let movement = (keys.pressed | keys.short | keys.repeat) & 3;
                if count != 0 && movement != 0 {
                    self.navigation.error_page = (self.navigation.error_page
                        + if movement & 1 != 0 { count - 1 } else { 1 })
                        % count;
                    return Actions::REDRAW;
                }
            }
            if keys.short & 4 != 0 {
                self.navigation.view = View::Menu;
                return Actions::REDRAW;
            }
            return Actions::empty();
        }
        if self.navigation.view == View::Menu {
            self.navigation.repeated |= (keys.pressed | keys.repeat) & 3;
            let movement = (keys.pressed | keys.short | keys.repeat) & 3;
            let direction = if movement & 1 != 0 {
                1
            } else if movement & 2 != 0 {
                -1
            } else {
                0
            };
            let item = self.item(self.navigation.selected);
            let mut actions = Actions::empty();
            if direction != 0 {
                if self.navigation.editing {
                    actions |= self.edit(item, direction);
                } else {
                    self.navigation.selected =
                        (self.navigation.selected as i32 + self.count() as i32 - direction)
                            .rem_euclid(self.count() as i32) as usize;
                    self.navigation.confirm_reset = false;
                    actions |= Actions::REDRAW;
                }
            }
            if keys.short & 4 != 0 {
                actions |= self.activate(item);
            }
            return actions;
        }
        let mut actions = Actions::empty();
        if keys.short & 1 != 0 && self.assist < self.preferences.settings.levels {
            self.assist += 1;
            actions |= Actions::CONTROLS | Actions::REDRAW;
        }
        if keys.short & 2 != 0 && self.assist != 0 {
            self.assist -= 1;
            actions |= Actions::CONTROLS | Actions::REDRAW;
        }
        if keys.long & 1 != 0 {
            self.lights = !self.lights;
            actions |= Actions::CONTROLS | Actions::REDRAW;
        }
        if keys.short & 4 != 0 {
            self.navigation.view = if self.navigation.view == View::Dashboard {
                View::Ride
            } else {
                View::Dashboard
            };
            actions |= Actions::REDRAW;
        }
        actions
    }
}
