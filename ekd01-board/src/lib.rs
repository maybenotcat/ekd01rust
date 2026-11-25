#![no_std]
//! EKD01 N22 board wiring, device profiles and startup policy.
pub mod backlight;
pub mod boot;
pub mod display;
mod display_profile;
pub mod error;
pub mod flash_profile;
pub mod memory;
mod panels;
pub mod pins;
pub mod power;
mod power_profile;
pub mod rtc;

pub mod bridge_uart;
pub mod keys;
