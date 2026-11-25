#![no_std]
#![allow(non_upper_case_globals, non_snake_case)]
//! Shared MT2502 hardware operations. Final binaries own startup and locking.
pub mod flash;
pub mod mmio;
pub mod pmu;
pub mod registers;

// Temporary extraction bridge: statically linked, supplied by each binary.
// Removed when diagnostics become operation results in the second stage.
unsafe extern "Rust" {
    fn mt2502_diagnostic_hex(name: &str, value: u32);
}
struct DebugUart;
impl DebugUart {
    fn hex(name: &str, value: u32) {
        unsafe { mt2502_diagnostic_hex(name, value) }
    }
}