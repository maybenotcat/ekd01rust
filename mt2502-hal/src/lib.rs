#![no_std]
#![allow(non_upper_case_globals, non_snake_case)]
//! Shared MT2502 hardware operations. Final binaries own startup and locking.
pub mod flash;
pub mod mmio;
pub mod pmu;
pub mod registers;

#[cfg(target_arch = "arm")]
pub mod boot;
pub mod btif;
#[cfg(target_arch = "arm")]
pub mod cpu;
pub mod debug_uart;
#[cfg(target_arch = "arm")]
pub mod display;
pub mod environment;
pub mod gpio;
pub mod gpt;
pub mod interrupt;
pub mod key_matrix;
pub mod memory;
#[cfg(target_arch = "arm")]
mod panels;
pub mod pwm;
mod ring;
pub mod rtc;
pub mod uart3;
pub mod watchdog;
