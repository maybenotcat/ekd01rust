#![no_std]
#![allow(non_upper_case_globals, non_snake_case)]
//! Shared MT2502 hardware operations. Final binaries own startup and locking.
pub mod flash;
pub mod mmio;
#[cfg(all(feature = "register-tests", target_arch = "arm"))]
compile_error!("register-tests is a native-only register model, never a firmware backend");
#[cfg(feature = "register-tests")]
extern crate std;
pub mod pmu;
pub mod registers;
#[cfg(feature = "register-tests")]
pub mod testing;

pub mod btif;
pub mod cache;
#[cfg(target_arch = "arm")]
pub mod cpu;
pub mod debug_uart;
pub mod environment;
pub mod gpio;
pub mod gpt;
pub mod interrupt;

pub mod psram;
pub mod pwm;
mod ring;
pub mod rtc;
pub mod uart3;
pub mod watchdog;

pub mod clocks;
pub mod emi;
pub mod error;
pub mod lcd;
pub mod memory_map;
pub mod sfi;

pub mod dma;
