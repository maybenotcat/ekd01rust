# EKD01 Rust Firmware

Experimental firmware for the Bafang EKD01 CAN-bus display, written in Rust without the MediaTek SDK.

Prototypes of all functions, including BLE, are implemented. The firmware has not been fully tested. The main screen layout still needs work. There are two main screen layouts. Both look bad.

Uses Embassy, Slint, Trouble and littlefs_rust. The auxiliary MCU firmware is left unchanged.

Tested on a display that originally ran `EKD01_CAN_BF_N22`. This may matter for compatibility: the auxiliary MCU firmware is not updated, and devices running other firmware versions have not been tested.

## Build

Install Rust through rustup. On Windows, install MSVC Build Tools and the Windows SDK.

```sh
cargo xtask build
```

The flash kit is generated in `target/flash-kit`.

## Flash

On Windows, install `drivers/windows/InstallDriver.exe`.

```sh
cargo flasher download --kit target/flash-kit
```

Start the command, then connect the display via USB with main power disconnected. The flasher selects the first newly appearing serial port.

By default, only the application is flashed. Existing bootloaders are preserved. To also flash the original bootloaders, add `--bootloaders`:

```sh
cargo flasher download --kit target/flash-kit --bootloaders
```

A bad firmware image can be recovered through the chip's built-in BootROM, even if the flash bootloaders are erased. Firmware flashing does not overwrite BootROM.

## BLE client

A Rust BLE telemetry client example is in [`ble-client/`](ble-client/). It has a desktop UI for scanning, connecting and viewing live telemetry.

```sh
cargo build-ble-client
cargo ble-client
```

Enable Bluetooth on the display, scan and connect. The client is read-only. On Linux, it uses BlueZ; building also requires `pkg-config` and `libdbus-1-dev`.

## EKD01 CAN PCB

![Display PCB photo 1](images/pcb1.jpg)

![Display PCB photo 2](images/pcb2.jpg)
