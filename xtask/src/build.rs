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
    Ok(())
}
