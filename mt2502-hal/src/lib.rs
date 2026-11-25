#![no_std]
#![allow(non_upper_case_globals, non_snake_case)]
//! Shared MT2502 hardware operations. Final binaries own startup and locking.
pub mod flash;
pub mod mmio;
pub mod pmu;
pub mod registers;
