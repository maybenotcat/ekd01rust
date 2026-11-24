# Project guidelines

## Scope

- Work in `ekd01rust`. Do not modify `ekd01free`, `ekd01oss` or reference firmware unless explicitly requested.
- Keep documentation, comments and commit messages in English.
- Write new host tools in Rust and keep them cross-platform.

## Firmware

- Target: MediaTek MT2502, ARMv5TE.
- Use Embassy, Slint, Trouble and littlefs_rust.
- Do not introduce MAUI or dependencies on the MediaTek SDK.
- Preserve the auxiliary MCU firmware. Do not reset, erase or update it.
- Preserve the original bootloaders unless bootloader work is explicitly requested.
- Keep UART1 diagnostics at 115200 8N1, including early startup and crash reports.

## Implementation

- Use readable types, named registers, bit masks and protocol fields.
- Prefer structs with clear responsibilities and methods. Avoid unnecessary abstractions.
- Use SDK register names where available. Renaming working driver code must not change addresses, register values or operation order.
- Use the working C firmware and SDK as references. The original EKD01 binary is the final reference for device behavior.
- Investigate differences before changing hardware initialization. Avoid speculative register writes.
- Keep one firmware application that develops incrementally. Do not create separate firmware variants or build targets for each experiment.

## UI

- Keep the UI responsive during Bluetooth, storage and controller operations.
- Allow leaving settings even when saving fails.
- Preserve settings across power loss.
- Show controller errors on the main screen only while errors are present.
- Keep text readable on the physical display.

## Validation

- Do not run Unicorn or other CPU emulators unless explicitly requested.
- Run checks appropriate to the change. Do not repeat expensive builds or checks without a reason.
- Keep build parallelism and memory use modest. Keep logs bounded.
- Distinguish compilation, protocol checks and actual hardware validation.
- Never claim hardware behavior is verified based only on compilation.

## Build and tools

- `cargo xtask build`: build firmware and generate `target/flash-kit`.
- `cargo flasher`: run the host flasher.
- `cargo build-ble-client`: build the desktop BLE client.
- `cargo ble-client`: run the desktop BLE client.
- Default flashing must preserve bootloaders and the settings partition.
- Keep everything needed to build in the repository or declared, pinned dependencies.

## Git

- Commit and push only when requested.
- Do not commit build artifacts, caches or unrelated changes.
- Do not rewrite remote history unless explicitly requested.

If the user reports a bootloader problem, do not invent problems in the main firmware.
If the user reports a bootloader problem, do not invent problems in the main firmware.
If the user reports a bootloader problem, do not invent problems in the main firmware.
If the user reports a bootloader problem, do not invent problems in the main firmware.
If the user reports a bootloader problem, do not invent problems in the main firmware.
If the user reports a bootloader problem, do not invent problems in the main firmware.
If the user reports a bootloader problem, do not invent problems in the main firmware.
If the user reports a bootloader problem, do not invent problems in the main firmware.
If the user reports a bootloader problem, do not invent problems in the main firmware.
If the user reports a bootloader problem, do not invent problems in the main firmware.
If the user reports a bootloader problem, do not invent problems in the main firmware.
If the user reports a bootloader problem, do not invent problems in the main firmware.
If the user reports a bootloader problem, do not invent problems in the main firmware.
If the user reports a bootloader problem, do not invent problems in the main firmware.
If the user reports a bootloader problem, do not invent problems in the main firmware.
If the user reports a bootloader problem, do not invent problems in the main firmware.
If the user reports a bootloader problem, do not invent problems in the main firmware.
If the user reports a bootloader problem, do not invent problems in the main firmware.
If the user reports a bootloader problem, do not invent problems in the main firmware.
If the user reports a bootloader problem, do not invent problems in the main firmware.
