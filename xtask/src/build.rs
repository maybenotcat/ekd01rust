//! Build the single firmware target, then leave packaging to the caller.
use crate::Result;
use std::{path::Path, process::Command};

pub fn firmware(root: &Path, offline: bool) -> Result<()> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    command.current_dir(root).args([
        "build",
        "--release",
        "--locked",
        "--package",
        "ekd01rust",
        "--target",
        "armv5te-none-eabi",
    ]);
    if offline {
        command.arg("--offline");
    }
    let status = command
        .status()
        .map_err(|error| format!("Cannot start Cargo: {error}"))?;
    if !status.success() {
        return Err(format!("Firmware build failed: {status}"));
    }
    // Thumb-1 keeps the ROM-loaded BL below the reserved SRAM stack. Its
    // reset/exception vectors and CPSR veneers remain explicitly ARM state.
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    command.current_dir(root).args([
        "build",
        "--release",
        "--locked",
        "--package",
        "ekd01-bootloader",
        "--target",
        "thumbv5te-none-eabi",
    ]);
    if offline {
        command.arg("--offline");
    }
    let status = command
        .status()
        .map_err(|error| format!("Cannot start Cargo: {error}"))?;
    if !status.success() {
        return Err(format!("Bootloader build failed: {status}"));
    }
    Ok(())
}
