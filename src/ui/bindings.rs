//! Exhaustive translations from navigation types to the Slint data contract.
use super::{
    MenuItem, MenuPage, Screen,
    data::Item,
    model::{Page, View},
};

impl From<Item> for MenuItem {
    fn from(value: Item) -> Self {
        match value {
            Item::Display => Self::Display,
            Item::Motor => Self::Motor,
            Item::Information => Self::Information,
            Item::FactoryReset => Self::FactoryReset,
            Item::Exit => Self::Exit,
            Item::Brightness => Self::Brightness,
            Item::Units => Self::Units,
            Item::AutoOff => Self::AutoOff,
            Item::Trip => Self::Trip,
            Item::Hour => Self::Hour,
            Item::Minute => Self::Minute,
            Item::Levels => Self::Levels,
            Item::Limit => Self::Limit,
            Item::Wheel => Self::Wheel,
            Item::Circumference => Self::Circumference,
            Item::BatteryInfo => Self::BatteryInfo,
            Item::CtrlInfo => Self::CtrlInfo,
            Item::DisplayInfo => Self::DisplayInfo,
            Item::TorqueInfo => Self::TorqueInfo,
            Item::Errors => Self::Errors,
            Item::Diagnostics => Self::Diagnostics,
            Item::Service => Self::Service,
            Item::Sensitivity => Self::Sensitivity,
            Item::Back => Self::Back,
            Item::Bluetooth => Self::Bluetooth,
            Item::Advanced => Self::Advanced,
            Item::Current => Self::Current,
            Item::Speed => Self::Speed,
            Item::Level1 => Self::Level1,
            Item::Level2 => Self::Level2,
            Item::Level3 => Self::Level3,
            Item::Level4 => Self::Level4,
            Item::Level5 => Self::Level5,
            Item::Level6 => Self::Level6,
            Item::Level7 => Self::Level7,
            Item::Level8 => Self::Level8,
            Item::Level9 => Self::Level9,
            Item::Start => Self::Start,
            Item::Language => Self::Language,
            Item::Theme => Self::Theme,
            Item::ServiceInterval => Self::ServiceInterval,
            Item::ServiceReset => Self::ServiceReset,
            Item::ErrorClear => Self::ErrorClear,
            Item::PowerMode => Self::PowerMode,
            Item::ServiceDistance => Self::ServiceDistance,
            Item::Odometer => Self::Odometer,
            Item::About => Self::About,
        }
    }
}

impl From<Page> for MenuPage {
    fn from(value: Page) -> Self {
        match value {
            Page::Root => Self::Root,
            Page::Display => Self::Display,
            Page::Motor => Self::Motor,
            Page::Information => Self::Information,
            Page::Advanced => Self::Advanced,
            Page::Current => Self::Current,
            Page::Speed => Self::Speed,
            Page::Start => Self::Start,
            Page::Service => Self::Service,
        }
    }
}

impl From<View> for Screen {
    fn from(value: View) -> Self {
        match value {
            View::Dashboard => Self::Dashboard,
            View::Ride => Self::Ride,
            View::Menu => Self::Menu,
            View::Diagnostics => Self::Diagnostics,
            View::Information => Self::Information,
        }
    }
}
