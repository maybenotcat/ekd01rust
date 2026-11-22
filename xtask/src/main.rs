//! Host-side build and packaging commands. No Python is required.
mod build;
mod elf;
mod package;

use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};
type Result<T> = std::result::Result<T, String>;
fn read(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))
}

fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut args = std::env::args_os().skip(1);
    let command = args.next().unwrap_or_else(|| "help".into());
    if matches!(command.to_str(), Some("help" | "--help" | "-h")) {
        println!(
            "cargo xtask build [--offline] [--out DIRECTORY]\n\
                  cargo xtask pack [--elf FILE] [--out DIRECTORY] [--identity FILE]\n\n\
                  build: compile release firmware and create a flash kit\n\
                  pack: package an existing ELF without rebuilding\n\
                  Default kit: target/flash-kit; all paths are relative to the current directory."
        );
        return Ok(());
    }
    let build = match command.to_str() {
        Some("build") => true,
        Some("pack") => false,
        _ => {
            return Err(format!(
                "Unknown command: {} (use cargo xtask help)",
                command.to_string_lossy()
            ));
        }
    };
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        })
        .unwrap_or_else(|| root.join("target"));
    let mut elf = target.join("armv5te-none-eabi/release/ekd01rust");
    let mut output = target.join("flash-kit");
    let mut identity = root.join("build/generated/build-id.json");
    let mut offline = false;
    while let Some(argument) = args.next() {
        match argument.to_str() {
            Some("--offline") if build => offline = true,
            Some("--elf") if !build => {
                elf = args.next().map(PathBuf::from).ok_or("Missing --elf FILE")?
            }
            Some("--out") => {
                output = args
                    .next()
                    .map(PathBuf::from)
                    .ok_or("Missing --out DIRECTORY")?
            }
            Some("--identity") if !build => {
                identity = args
                    .next()
                    .map(PathBuf::from)
                    .ok_or("Missing --identity FILE")?
            }
            _ => return Err(format!("Unknown option: {}", argument.to_string_lossy())),
        }
    }
    if build {
        build::firmware(root, offline)?;
    }
    package::pack(root, &elf, &output, &identity)
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}
