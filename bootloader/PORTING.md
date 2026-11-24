# MT2502 single bootloader

Boot path: BootROM -> EKD01_BOOT.bin -> EKD01RUST.bin. No original BL or EXT_BL code runs after installing the custom loader. The original binary USB Download Agent is still used by the host flasher, only during USB flashing.

The user confirmed successful device startup after restoring the original RTC first-power-on path (`0815` -> `1002cf1c`). This path initializes retained keys, low-power detectors and IRQ/alarm state; it is more than the calendar-only routine at `1002cf00`. Update swapping, interrupted updates and rollback still require hardware validation. Earlier investigation notes below record intermediate failures, not the current startup result.

## Memory

Physical NOR addresses:

| Region | Start | End, exclusive |
| --- | --- | --- |
| BootROM boot metadata and custom loader | 0x000000 | below 0x050000 |
| Active GFH application | 0x050000 | 0x250000 |
| DFU application and swap scratch page | 0x250000 | 0x451000 |
| Embassy state | 0x451000 | 0x455000 |
| Reserved | 0x455000 | 0x500000 |
| Existing settings/data | 0x500000 | 0x800000 |

`boot-contract` defines the common physical layout. The application linker additionally enforces the 2MiB active-slot limit. DFU has one extra 4KiB erase page. State has 16KiB, enough for four progress words per active page with 4-byte writes.

BootROM loads the loader header to 0x70006000 and enters ARM code at 0x70006140. The complete file, including GFH and hash, must end at or below 0x70009000. The original SRAM stack remains at 0x70009000..0x7000a000; linker and packer both enforce the boundary. The previous 8KiB cap was our conservative assumption based on the original 8180-byte file, not a decoded ROM limit. Readback traced the ROM rejection to BRLYT attributes. Hardware UART subsequently confirmed ROM acceptance and application handoff with the enlarged 10040-byte image. No XIP code is called while NOR is busy. The application later reuses SRAM for its existing flash routines and IRQ stack.

`cargo xtask build` builds the application for `armv5te-none-eabi` and the loader for `thumbv5te-none-eabi`. Both target the same ARMv5TE processor. ARM reset/exception code and CPSR veneers interwork with Thumb-1 Rust code, keeping a single loader within the ROM-load budget. The loader ELF is at `target/thumbv5te-none-eabi/release/ekd01-bootloader`.

The loader passes its magic, inverse, version and trial flag in R4..R7. Application entry preserves these registers and captures them into a record at uncached PSRAM 0x0003f000, outside application allocations and low vectors. It does not trust a stale PSRAM record left by a previous warm boot. The application only writes confirmation when the handoff is valid and a trial is pending; booting through a legacy loader does not touch Embassy state.

## Original reference

The original N22 ARM_BL runtime ELF was inspected in IDA:

- 0x70006684: PLL/clock sequence, including the USB-specific preliminary clock switch.
- 0x700065e4: bus DCM and package-specific pad setup.
- 0x70006f10: EMI register programming, remap, DLL and arbitration setup.
- 0x70006ed0 and 0x70006e3a: delay-tap calibration and the hardware EMI BIST sequence.
- 0x70006a78: UART1 divider 28, 8N1 setup.
- Original EXT_BL `bl_FUNET_LatchPower`: RTC oscillator restore, RTC initialization and RTC-controlled power latch. SDK register definitions identify these operations; no SDK objects are linked.
- EXT_BL ROM 0x1004a238..0x1004c9ac is copied to SRAM 0x70005000..0x70007774. Its custom_setSFIExt at runtime 0x70005142 calls configuration 0x70005098. GD25LQ64 is table index 2, selected by 0x10033bf2 using the JEDEC table at 0x10049a80. The port issues the same volatile status/QPI/read-parameter commands, six HWConf_130M register writes, DCM driving fields and SFI clock switch. The physical boot header is compared with its SRAM copy after switching.

The original 0x140-byte BL GFH supplies the BootROM configuration, security policy, ARM_BL_INFO and exact EMI_LIST values. File length and checksum are regenerated; all other original GFH records are preserved. The 32-byte HASH trailer is SHA-1 over the header and payload followed by twelve zero bytes, matching the original format. It is an integrity checksum, not an asymmetric signature. The application remains unsigned; structural GFH/vector/bounds checks are performed before handoff.

The first 10664-byte ARM build was rejected before handoff with `00: 102C 0001`, `01: 102C 0000`, and `Boot failed, reset ...`. Its host SHA-1 was valid. That log alone did not establish a load-window limit. The smaller image below was rejected for the same independently identified metadata error.

The smaller 7976-byte loader was also rejected. Device readback then proved
that its bytes and SHA-1 were correct, but BRLYT at 0x200 contained boot
attributes 0x140 at 0x224 instead of GFH_BL_INFO's 1. Original DA 0x1002918e
receives the third descriptor word into structure offset 16; 0x10029690
copies that word to BRLYT.m_bl_attribute. The flasher had sent jump_offset
there, clearing BL_ATTR_LOAD_BY_BOOTROM. It now sends the actual BL_INFO
attribute (1 for ARM_BL, 0 for original ARM_EXT_BL) and validates the DA-created
metadata after writing. The size reduction alone did not fix the rejection.

The loader owns PLL, PSRAM, UART, low exception vectors, RTC power retention and the internal watchdog. The application continues to initialize PMU calibration, LCD, buttons, MCU bridge and Bluetooth with its existing drivers. The auxiliary MCU is neither reset nor updated. FAT, Redstone, MAUI initialization and the old EXT_BL handoff services are not used.

## Update behavior

The upstream Embassy Boot 0.7.0 blocking swap algorithm runs without a scheduler. The loader uses the same tested SFI manual-transfer implementation as the application. Each partition has an explicit write window. Settings calls retain their original lower write boundary.

Boot partition writes check the detected JEDEC ID independently of active GFH validity: a power cut can leave the active header erased, and recovery must still be able to finish the swap. Ordinary settings initialization keeps its original active-header check.

After a successful swap, the first completed Slint/LCD frame confirms the trial. This proves the application reached its executor and UI/DMA path; it does not prove motor or BLE connectivity. If no confirmation is written, a subsequent reset reverts. Watchdog starvation also resets the application. A hang that still services the watchdog needs a manual reset to trigger rollback.

`cargo flasher download --kit target/flash-kit --bootloaders` installs the custom loader and application through BootROM/DA. Later direct downloads preserve the loader unless requested and clear only the Embassy state after image verification. `stage` first verifies that the matching custom loader is installed, refuses an existing pending trial, erases the spare slot, writes and verifies the new image, and writes the pending marker last. It preserves active firmware until the next boot.

The generated scatter has two executable files: one ARM_BL and one application. It contains no EXT_BL entry. The DA boot-region command sends one descriptor and creates SF_BOOT/BRLYT metadata; a RAM load address is never used as a physical flash offset.

## Validation

Native tests cover slot boundaries, GFH/checksum packaging, one-loader DA metadata, full-size swap/confirmation, and power cuts during programming and partial erases while swapping/reverting. These are flash-algorithm/protocol tests, not a CPU emulator. Firmware and loader are compiled for ARMv5TE. Hardware UART output now confirms BootROM acceptance, custom loader execution, PSRAM/SFI initialization and application handoff. Full application startup, staged updates and rollback still require hardware validation with this loader.

The first hardware run stopped logging after LCD supply enable. A subsequent diagnostic build (62010d5061f9862e) reported working GPT1 IRQs both before and after PMU initialization, identified panel 77895200, completed the first LCD DMA transfer and received MCU/CAN traffic. The added waits changed timing; that run does not prove the original initialization was complete. The hardware parity changes below have been compiled but still need device testing.

## Hardware initialization audit

The reference is the original N22 ARM_BL and EXT_BL executable path, not every optional SDK feature. Addresses below refer to original runtime code.

| Original path | Rust implementation / disposition |
| --- | --- |
| BL pad/DCM and PLL initialization, 700065e4 / 70006684 | `soc::initialize_clocks`, original operation order |
| BL EMI timings, calibration/BIST and DLL, 70006f10 / 70006ed0 / 70006e3a | `emi::initialize`, timings from original GFH |
| BL UART1, 70006a78 | `soc::initialize_uart`, 115200 8N1 |
| BL F32K default initialization, 70006ac4 | `rtc::initialize_oscillator`, RTC keys, low-power flag and silicon-revision branches |
| BL 32k startup/stability, 700069c0 / 70006ba2 / 70006bac | `soc::stabilize_32k`, MD clock start, USCNT, original measurement/window arithmetic |
| BL conditional DMA ungating, 700062b2 | N22 GFH feature word is 0x560; mask 0x4 is clear, so the original skips this write |
| EXT_BL SFI, copied runtime 70005142 / 70005098 | `sfi::initialize`, original GD25LQ64 device commands, timing and clock fields |
| EXT_BL flash geometry / FTL / FAT, 10028030 / 1002811c | NOR geometry is checked by `Flash`; FAT/FTL initialization is replaced by bounded Embassy partitions, never mounted or formatted |
| EXT_BL charger setup, 10027fb0 / 10027fd0 | `soc::initialize_power`, 7V detection threshold, original USB-without-battery branch and conditional charge/current setup |
| EXT_BL PMU startup, 10028358 / 10028ca4 | Same shared `pmu_data::STARTUP` table used by the application; original conditional execution and ADIE interrupt enable |
| EXT_BL board PMU callback, 10028e10 | KPLED mode, speaker gain, ISINK mode/current, long-press shutdown and trigger pulse |
| EXT_BL RTC_setXOSC, 1002f57c / 10031a50 / 10031b10 | `rtc::restore_running_clock`, original trim search and long-window tie-break, frequency measurement, RTC correction and supported silicon-mode switch |
| EXT_BL RTC_HW_Init and latch, 1002f38c / 1002f398 | RTC reload, BBPU latch, 2ms debounce and weekday/spare preservation |
| EXT_BL cache disable / UART drain / remap / jump | Cache disabled before NOR operations and handoff, UART transmit drain, remap 3 established by EMI, ARM jump with explicit application handoff |
| Original security/MAUI pairing, Redstone/FOTA and BL shared-service objects | Not reused: original GFH ROM policy retained; application structure validated and update/revert handled by Embassy Boot |

No original code or SDK object is called. The auxiliary MCU is not reset or updated. Extra PSRAM probes, watchdog bounds and readable failure reports are retained. Polling loops that were effectively unbounded in the original halt with a UART error when their hardware does not respond.

### Watchdog register-width correction

Original BL 7000622a/700062a6/700062b0 writes the restart key using STR (32-bit). MT6261 enables DRV_WDT_RETN_REG; SDK WDT_Restart2 explicitly uses DRV_WDT_WriteReg32 for this family. The port incorrectly used 16-bit stores to WDT_RESTART in the loader, application entry, application watchdog and SRAM flash wait/error paths. All restart accesses now use 32-bit stores. WDT_MODE and WDT_LENGTH remain 16-bit registers. This corrects a documented register-access mismatch; whether it caused the observed shutdown still needs hardware verification. It does not establish the cause of the LCD-ID startup stop.

### Second comparison after the LCD-ID startup stop

The 10040-byte loader passes 32k stability, SFI setup and power latching, and the application passes both GPT IRQ checks. The last reported application phase is READ_ID_ENTER. This does not identify the blocking instruction and does not establish a panel fault.

The second instruction-level comparison found and restored these ordering differences:

- BL 70006240 -> 70006e0c writes BOOT_CON=2 before PLL/pad initialization. The port previously waited until EMI initialization.
- EXT_BL 70005098 repeats the complete bus DCM and package-pad setup after EMI calibration. The port previously performed that setup only once before PLL initialization.
- EXT_BL 70005158..7000517e clears the SFI clock selection and sets the selected divider in separate writes. It chooses divider 1 for MPLL code 0x14 and divider 2 for 0x18. The port previously combined clear/set and hardcoded divider 1.
- SRAM handoff 7000505a repeats BOOT_CON=3 immediately before JumpCmd. The port previously relied only on the earlier EMI remap write.

BL BIST patterns, loop directions, delay-tap search, GFH timing offsets, DLL and arbitration values were rechecked against executable instructions. EXT_BL scatter loading copies data and clears BSS; its flash/FTL initialization sets up geometry and software objects. The GD25LQ64 command list has no status-poll entry omitted by the port. The above corrections are verified differences, not a hardware-confirmed explanation for the LCD stop. No additional LCD probe code or timer watchdog is included in this correction.

Expected early UART1 sequence at 115200 8N1:

```text
[BOOT] EKD01 EMBASSY; single loader; UART1 115200
[BOOT] EMI_INIT_ENTER
[BOOT] EMI_INIT_EXIT
[BOOT] 32K_STABLE
[BOOT] EMI_GENA=00000011
[BOOT] PSRAM_OK
[BOOT] SFI_INIT_ENTER
[FLASH] JEDEC=00C86017
[BOOT] SFI_DIRECT_CTL=0C0B5771
[BOOT] CLK_CONDC=0000B621
[BOOT] SFI_INIT_EXIT
[BOOT] PMU_READY
[BOOT] POWER_LATCHED
[BOOT] ACTIVE
[BOOT] JUMP=10050520
[RUST] ENTRY; UART1 115200 inherited
```

A staged update prints `SWAPPED; trial application`, followed by `TRIAL_CONFIRMED` after the first displayed frame. A failed/unconfirmed trial prints `REVERTED` on the subsequent boot. Boot exception reports include exception kind, LR and SPSR. Boot failures halt with readable UART diagnostics; physical USB BootROM recovery remains available.
