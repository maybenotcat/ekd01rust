//! Native cross-platform MT2502 USB flasher with the original binary DA.
mod agent;
mod brom;
mod da;
mod images;
mod port;
mod wire;

use std::{fs::OpenOptions, io::Write, path::PathBuf, process::ExitCode, time::Duration};
type Result<T> = std::result::Result<T, String>;

fn number(text: &str) -> Result<u32> {
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16)
    } else {
        text.parse()
    }
    .map_err(|e| format!("Invalid number {text}: {e}"))
}
fn help() {
    println!(
        "EKD01 Rust USB flasher (original Varstrom RAM agent)\n\n\
        cargo flasher ports\n\
        cargo flasher plan --kit target/flash-kit [--bootloaders]\n\
        cargo flasher probe [--port COM3]\n\
        cargo flasher download --kit target/flash-kit [--port COM3] [--bootloaders] [--no-reset]\n\
        cargo flasher stage --kit target/flash-kit [--port COM3] [--no-reset]\n\
        cargo flasher verify --kit target/flash-kit [--port COM3] [--bootloaders]\n\
        cargo flasher readback --output backup.bin [--port COM3] [--start 0x0 --length 0x800000]\n\
        cargo flasher format --all [--port COM3]\n\
        cargo flasher format --start 0x500000 --length 0x60000 [--port COM3]\n\n\
        USB wait defaults to 60 seconds (--wait SECONDS). Without --port, existing\n\
        ports are ignored and the first newly appearing port is selected (any VID/PID).\n\
        Reconnecting an existing port also works. Explicit --port accepts /dev/ttyACM0.\n\
        Download writes only the main application unless --bootloaders is selected.\n\
        Embassy kits also reset rollback state after verified direct download.\n\
        Stage keeps the active image and marks a verified spare-slot update pending.\n\
        Format is separate; --all erases the entire 8 MiB, including settings and loaders.\n\
        Every download is verified by reading back the complete written images.\n\
        Native implementation; no Flash_tool.exe, DLLs, Python or auxiliary MCU update."
    );
}
fn run() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let operation = args.next().unwrap_or_else(|| "help".into());
    let operation = operation.to_str().ok_or("Invalid command")?;
    if matches!(operation, "help" | "-h" | "--help") {
        help();
        return Ok(());
    }
    if ![
        "ports", "plan", "probe", "download", "stage", "verify", "readback", "format",
    ]
    .contains(&operation)
    {
        return Err("Unknown command; use cargo flasher help".into());
    }
    let mut kit = None;
    let mut output = None;
    let mut port = None;
    let mut start = None;
    let mut length = None;
    let mut wait = 60u32;
    let mut bootloaders = false;
    let mut reset = true;
    let mut all = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--kit") => kit = Some(PathBuf::from(args.next().ok_or("Missing --kit path")?)),
            Some("--output") => {
                output = Some(PathBuf::from(args.next().ok_or("Missing --output path")?))
            }
            Some("--port") => {
                port = Some(
                    args.next()
                        .ok_or("Missing --port name")?
                        .into_string()
                        .map_err(|_| "Invalid port name")?,
                )
            }
            Some("--start" | "--length" | "--wait") => {
                let value = number(
                    &args
                        .next()
                        .ok_or("Missing numeric argument")?
                        .to_string_lossy(),
                )?;
                match arg.to_str().unwrap() {
                    "--start" => start = Some(value),
                    "--length" => length = Some(value),
                    _ => wait = value,
                }
            }
            Some("--bootloaders") => bootloaders = true,
            Some("--no-reset") => reset = false,
            Some("--all") => all = true,
            _ => return Err(format!("Unknown option {}", arg.to_string_lossy())),
        }
    }
    if operation == "ports" {
        return port::list();
    }
    if wait == 0 || wait > 3600 {
        return Err("--wait must be 1..3600 seconds".into());
    }
    if all && (start.is_some() || length.is_some()) {
        return Err("Use --all or a range, not both".into());
    }
    if all && operation != "format" {
        return Err("--all is only valid for format".into());
    }
    if bootloaders && !["plan", "download", "verify"].contains(&operation) {
        return Err("--bootloaders requires plan, download or verify".into());
    }
    let selected = if ["plan", "download", "stage", "verify"].contains(&operation) {
        let kit = images::Kit::load(&kit.ok_or("Specify --kit directory-or-scatter")?)?;
        if operation == "stage" {
            let staged = kit.staged_application()?;
            println!(
                "STAGE {:08X}..{:08X} {}; active and settings preserved",
                staged.physical,
                staged.physical + staged.bytes.len() as u32,
                staged.name
            );
        } else {
            kit.print(bootloaders);
        }
        Some(kit)
    } else {
        None
    };
    agent::validate()?;
    if operation == "plan" {
        println!("PLAN ONLY; no device connection");
        return Ok(());
    }
    let (begin, count) = match operation {
        "format" if all => (0, images::FLASH_BYTES),
        "format" => (
            start.ok_or("Format requires --all or --start and --length")?,
            length.ok_or("Format requires --length")?,
        ),
        "readback" => (start.unwrap_or(0), length.unwrap_or(images::FLASH_BYTES)),
        _ => (0, 0),
    };
    if ["format", "readback"].contains(&operation)
        && (count == 0
            || begin
                .checked_add(count)
                .is_none_or(|end| end > images::FLASH_BYTES))
    {
        return Err("Selected range is outside 8 MiB NOR".into());
    }
    if operation == "format" && (begin % 4096 != 0 || count % 4096 != 0) {
        return Err("Format requires a 4 KiB aligned range".into());
    }
    // create_new prevents silently overwriting an existing backup.
    let mut destination = if operation == "readback" {
        let path = output.ok_or("Readback requires --output FILE")?;
        Some((
            path.clone(),
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|e| format!("{}: {e}", path.display()))?,
        ))
    } else {
        None
    };
    let timeout = Duration::from_secs(wait as u64);
    let transport = port::wait(port.as_deref(), timeout)?;
    let mut brom = brom::BootRom::connect(transport, timeout)?;
    let chip = brom.chip()?;
    chip.print();
    if chip.code != 0x6261 {
        return Err(format!(
            "Original DA requires MT6261/MT2502 silicon code 6261, got {:04X}",
            chip.code
        ));
    }
    let mut agent = da::DownloadAgent::initialize(brom.start_agent()?)?;
    match operation {
        "probe" => println!("PROBE OK"),
        "format" => {
            println!("FORMAT {:08X}..{:08X}", begin, begin + count);
            agent.format(begin, count)?;
            println!("FORMAT COMPLETE");
        }
        "readback" => {
            let (path, file) = destination.as_mut().unwrap();
            let mut bytes = 0;
            agent.read(begin, count, |block| {
                file.write_all(block).map_err(|e| e.to_string())?;
                bytes += block.len();
                Ok(())
            })?;
            file.sync_all().map_err(|e| e.to_string())?;
            println!("READBACK COMPLETE: {} ({bytes} bytes)", path.display());
        }
        "stage" => {
            let kit = selected.as_ref().unwrap();
            // A pending trial must boot/confirm or revert before its backup is
            // reused. Also prove that the matching custom loader is installed.
            agent.verify(&kit.images[0])?;
            agent.verify_boot_metadata(&kit.images[..kit.images.len() - 1])?;
            let state = ekd01_boot_contract::STATE;
            let mut magic = Vec::new();
            agent.read(state.start, 4, |bytes| {
                magic.extend_from_slice(bytes);
                Ok(())
            })?;
            if magic == ekd01_boot_contract::PENDING_MARKER {
                return Err("An update is already pending; boot the display first".into());
            }
            let dfu = ekd01_boot_contract::DFU;
            agent.format(dfu.start, dfu.len())?;
            let staged = kit.staged_application()?;
            agent.application(&staged)?;
            agent.verify(&staged)?;
            // Mark pending last, after complete readback verification.
            agent.format(state.start, state.len())?;
            let marker = kit.pending_marker()?;
            agent.application(&marker)?;
            agent.verify(&marker)?;
            println!("STAGE COMPLETE; next boot swaps and requires application confirmation");
            if reset {
                agent.reset()?;
            }
        }
        "download" | "verify" => {
            let kit = selected.as_ref().unwrap();
            if operation == "download" {
                if bootloaders {
                    agent.bootloaders(&kit.images[..kit.images.len() - 1])?;
                    agent.verify_boot_metadata(&kit.images[..kit.images.len() - 1])?;
                }
                agent.application(kit.images.last().unwrap())?;
            }
            if operation == "verify" && bootloaders {
                agent.verify_boot_metadata(&kit.images[..kit.images.len() - 1])?;
            }
            for image in kit
                .images
                .iter()
                .filter(|image| bootloaders || image.kind == 0x100)
            {
                agent.verify(image)?;
            }
            if operation == "download" && kit.embassy {
                let state = ekd01_boot_contract::STATE;
                agent.format(state.start, state.len())?;
                agent.read(state.start, state.len(), |bytes| {
                    if bytes.iter().all(|byte| *byte == 0xff) {
                        Ok(())
                    } else {
                        Err("Embassy state erase verification failed".into())
                    }
                })?;
                println!("BOOT STATE RESET; settings preserved");
            }
            println!("{} COMPLETE", operation.to_uppercase());
            if operation == "download" && reset {
                agent.reset()?;
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ERROR: {error}");
            ExitCode::FAILURE
        }
    }
}
