# MT2502 bootloader

Boot path: BootROM -> EKD01_BOOT.bin -> EKD01RUST.bin. The original BL and
EXT_BL are replaced. USB flashing still uses the original binary Download
Agent loaded by BootROM; the auxiliary MCU is neither reset nor updated.

## Current validation

The user confirmed successful device startup with this loader after the RTC
first-power-on sequence was restored. Staged updates, interrupted updates and
rollback still require hardware validation. Native flash fault-injection tests
check the generic swap algorithm, not physical peripheral behavior.

## Layout

Physical NOR offsets; ends are exclusive:

| Region | Start | End |
| --- | --- | --- |
| Boot metadata and loader | 000000 | below 050000 |
| Active application | 050000 | 250000 |
| DFU and swap scratch page | 250000 | 451000 |
| Update state | 451000 | 455000 |
| Reserved | 455000 | 500000 |
| Existing settings/data | 500000 | 800000 |

`boot-contract` owns these constants. DFU includes an extra 4 KiB scratch page.
The application remains at 10050520. Both linker and packer enforce bounds.

BootROM loads the GFH at 70006000 and enters ARM code at 70006140. The complete
loader, including its SHA-1 trailer, must end below 70009000; its stack top is
7000a000. Rust loader code uses Thumb-1 with ARM reset/exception/CPSR veneers.
The application replaces the loader's temporary SRAM contents after handoff.

## Platform initialization

The reference is the original N22 binary. The loader initializes PLL, EMI and
PSRAM calibration, UART1 at 115200 8N1, SFI and RTC/PMU power retention. It
retains the original ROM-facing GFH records and regenerates lengths and hash.
The GFH SHA-1 trailer is an integrity checksum, not an asymmetric signature.

The RTC path includes oscillator restoration, conditional first-power-on
initialization (command 0815 -> 1002cf1c), retained keys, low-power detector
reset, alarm/IRQ initialization, readback checks and BBPU power latching.

Common flash/MMIO/register/PMU data lives in the `mt2502-hal` crate, linked into both
firmware crates. N22 profiles, wiring, power policy and startup order live in the
shared `ekd01-board` crate. HAL returns typed errors; loader chooses stop policy.
Neither binary imports drivers from the other's source tree.
The application owns its scheduler, LCD, MCU bridge and Bluetooth initialization.
No SDK object, MAUI, FAT or Redstone code is linked.

## Build and identities

`cargo xtask build` builds the ARM application, Thumb loader and flash kit.
`cargo xtask pack` packages existing ELFs without rebuilding them.

Application and loader have independent BUILD_ID values, printed on UART1.
The application identity excludes loader sources. Changes to common drivers,
shared contract or dependency configuration can legitimately rebuild both.
`image.json` records both image hashes and the loader identity when its matching
generated metadata is available. Application and loader metadata are generated
separately under `build/generated/`.

`embassy-boot` is pinned to crates.io version 0.7.0 and left unchanged. The
local `embassy-hal-internal` patch supplies ARMv5-compatible atomics. Dependency
sources must be fetched once before offline builds can use the Cargo cache.

## Updates and recovery

The three bounded views share one physical NOR controller. The HAL implements `embedded-storage` traits for the
blocking Embassy Boot algorithm. Update progress is journaled in STATE.
The loader passes magic/inverse/version/trial in R4..R7; the application captures
it at physical PSRAM 0003f000. Only a valid trial handoff can confirm an update.
Confirmation follows the first successful LCD DMA frame; it does not certify
motor or BLE functionality. Unconfirmed trials revert on a subsequent reset.

`cargo flasher download --kit target/flash-kit --bootloaders` installs the
loader and application. Later direct downloads preserve loaders unless asked
and reset update state after verification. `stage` checks the installed loader,
refuses a pending update, verifies the spare image and writes the pending marker
last. Neither operation accesses the auxiliary MCU.

The application GFH/vector structure and bounds are checked before jumping.
A loader error prints UART diagnostics and halts; BootROM USB recovery remains
available. The watchdog resets a stalled application; a hang that continues to
feed it requires a manual reset before rollback can run.

Historical investigation notes are kept separately in `docs/`.
