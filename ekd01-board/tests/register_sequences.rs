//! Pre-refactor oracle captured before production algorithms are split.
use mt2502_hal::{registers::*, testing as io};
use mt2502_register_reference as reference;
use std::panic::{AssertUnwindSafe, catch_unwind};

struct Env;
impl mt2502_hal::environment::Trace for Env {
    fn log(_: &str) {}
    fn hex(_: &str, _: u32) {}
}
impl reference::environment::Trace for Env {
    fn log(_: &str) {}
    fn hex(_: &str, _: u32) {}
}
impl reference::environment::BootEnvironment for Env {
    fn stop(message: &str) -> ! {
        panic!("{message}")
    }
}

#[derive(Clone, Copy, Debug)]
enum Case {
    Uart,
    Power { usb: bool },
    Pads { package: u32 },
    Clocks { usb: bool },
    Clock32k,
    Clock32kBusy,
    Pmu { efuse: u32, usb: bool },
    RtcLatch { fresh: bool },
    RtcBusy,
    RtcLatchDenied,
    Oscillator { revision: u32 },
    Emi,
    EmiCalibrationFailed,
    Psram,
    Sfi { failure: u8 },
    BootPlatform,
    Nor { mode: u8 },
    Uart3,
    KeyMatrix,
    Dma { mode: u8 },
    Handoff,
    Watchdog,
    Cache,
    HeapProbe,
    PowerOff,
    Backlight,
    Panel { id: u32 },
    Charging,
    RtcEmbedded,
    RtcEmbeddedFresh,
    RtcNoClock,
    RtcCorrupt,
    RtcRollover,
    RtcReadyAfterBusy,
}
const CASES: &[(&str, Case)] = &[
    ("uart", Case::Uart),
    ("nor-program", Case::Nor { mode: 0 }),
    ("nor-busy-ready", Case::Nor { mode: 1 }),
    ("nor-erase", Case::Nor { mode: 2 }),
    ("rtc-embedded-fresh", Case::RtcEmbeddedFresh),
    ("rtc-no-clock", Case::RtcNoClock),
    ("panel-8552", Case::Panel { id: 0x00855200 }),
    ("panel-8901", Case::Panel { id: 0x00890100 }),
    ("panel-81b3", Case::Panel { id: 0x0081b300 }),
    ("panel-0181", Case::Panel { id: 0x01810000 }),
    ("panel-8966", Case::Panel { id: 0x00896600 }),
    ("uart3-mcu", Case::Uart3),
    ("key-matrix", Case::KeyMatrix),
    (
        "pmu-code26",
        Case::Pmu {
            efuse: (1 << 12) | (26 << 13),
            usb: false,
        },
    ),
    (
        "pmu-code27",
        Case::Pmu {
            efuse: (1 << 12) | (27 << 13),
            usb: false,
        },
    ),
    ("dma-complete", Case::Dma { mode: 0 }),
    ("dma-idle-timeout", Case::Dma { mode: 1 }),
    ("dma-start-timeout", Case::Dma { mode: 2 }),
    ("dma-bad-address", Case::Dma { mode: 3 }),
    ("dma-bad-count", Case::Dma { mode: 4 }),
    ("dma-bad-region", Case::Dma { mode: 5 }),
    ("dma-partial", Case::Dma { mode: 6 }),
    ("boot-platform", Case::BootPlatform),
    ("handoff", Case::Handoff),
    ("watchdog", Case::Watchdog),
    ("cache", Case::Cache),
    ("heap-probe", Case::HeapProbe),
    ("power-off", Case::PowerOff),
    ("backlight", Case::Backlight),
    ("panel-st7789", Case::Panel { id: 0x77895200 }),
    ("panel-unknown", Case::Panel { id: 0 }),
    ("sfi-ready", Case::Sfi { failure: 0 }),
    ("sfi-device", Case::Sfi { failure: 1 }),
    ("sfi-pll", Case::Sfi { failure: 2 }),
    ("sfi-header", Case::Sfi { failure: 3 }),
    ("power-charging", Case::Charging),
    ("rtc-embedded", Case::RtcEmbedded),
    ("rtc-corrupt", Case::RtcCorrupt),
    ("rtc-rollover", Case::RtcRollover),
    ("rtc-ready-after-busy", Case::RtcReadyAfterBusy),
    ("power-normal", Case::Power { usb: false }),
    ("power-usb", Case::Power { usb: true }),
    ("pads-package0", Case::Pads { package: 0 }),
    ("pads-package2", Case::Pads { package: 2 }),
    ("clocks-normal", Case::Clocks { usb: false }),
    ("clocks-usb", Case::Clocks { usb: true }),
    ("clock32k", Case::Clock32k),
    ("clock32k-busy", Case::Clock32kBusy),
    (
        "pmu-efuse0",
        Case::Pmu {
            efuse: 0,
            usb: false,
        },
    ),
    (
        "pmu-calibrated",
        Case::Pmu {
            efuse: 0x21884,
            usb: false,
        },
    ),
    (
        "pmu-usb",
        Case::Pmu {
            efuse: 0x21884,
            usb: true,
        },
    ),
    ("rtc-retained", Case::RtcLatch { fresh: false }),
    ("rtc-first-on", Case::RtcLatch { fresh: true }),
    ("rtc-busy", Case::RtcBusy),
    ("rtc-latch-denied", Case::RtcLatchDenied),
    ("osc-revision1", Case::Oscillator { revision: 1 }),
    ("osc-revision2", Case::Oscillator { revision: 2 }),
    ("emi", Case::Emi),
    ("emi-calibration-failed", Case::EmiCalibrationFailed),
    ("psram", Case::Psram),
];
fn setup(case: Case) {
    match case {
        Case::RtcEmbedded
        | Case::RtcEmbeddedFresh
        | Case::RtcNoClock
        | Case::RtcCorrupt
        | Case::RtcRollover
        | Case::RtcReadyAfterBusy => {
            setup(Case::RtcLatch { fresh: false });
            match case {
                Case::RtcEmbedded | Case::RtcEmbeddedFresh | Case::RtcNoClock => {
                    io::value(16, RTC_XOSCCAL, EKD_RTC_OSC_EMBEDDED);
                    io::value(32, 0x80000000, 2);
                    io::counter(32, 0xa0180104, 0, 128);
                    io::reads(16, PMU_base + 0xff8, [0x633, 0x633, 0x633, 51200]);
                    if matches!(case, Case::RtcEmbeddedFresh) {
                        io::value(16, RTC_base + 0x50, 0);
                        io::value(16, RTC_base + 0x54, 0);
                        io::fixed(16, PMU_base + 0xff8, 0x633);
                    }
                    if matches!(case, Case::RtcNoClock) {
                        io::fixed(32, 0xa0180104, 0);
                    }
                }
                Case::RtcCorrupt => io::value(16, RTC_base + 0x20, 0),
                Case::RtcRollover => io::reads(16, RTC_TC_SEC, [59, 0, 0]),
                Case::RtcReadyAfterBusy => {
                    io::reads(16, RTC_BBPU, [EKD_RTC_CBUSY, EKD_RTC_CBUSY, 0])
                }
                _ => unreachable!(),
            }
            return;
        }
        _ => {}
    }
    io::reset();
    configure(case);
}
fn configure(case: Case) {
    // Independent peripheral time: reads advance, rather than executing a CPU.
    io::counter(32, 0xa00c002c, 0, 128);
    match case {
        Case::Power { usb } | Case::Pmu { usb, .. } => io::value(
            16,
            EKD_BOOT_FLAGS,
            if usb { EKD_BOOT_USB_WITHOUT_BATTERY } else { 0 },
        ),
        _ => {}
    }
    match case {
        Case::Dma { mode } => {
            configure(Case::Panel { id: 0x77895200 });
            io::set_dma_address(if mode == 3 { 0 } else { 0x100000 });
            io::fixed(
                16,
                LCD_STA_REG,
                if mode == 1 { LCD_STATUS_RUN_BIT } else { 0 },
            );
            if mode != 2 {
                io::reads(16, LCD_INT_STATUS_REG, [0, 0, LCD_INT_STATUS_COMPLETE_BIT]);
            }
        }
        Case::BootPlatform => {
            configure(Case::Emi);
            configure(Case::RtcLatch { fresh: false });
            configure(Case::Clock32k);
            configure(Case::Sfi { failure: 0 });
            for (i, value) in [0x11, 0x10, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
                .into_iter()
                .enumerate()
            {
                io::value(32, 0x10000800 + 0xdc + i as u32 * 4, value);
            }
        }
        Case::Handoff | Case::PowerOff => io::fixed(16, UART_LSR(UART1_base), UART_LSR_TEMT),
        Case::Panel { id } => io::reads(8, LCD_SERIAL_DATA0_REG, id.to_be_bytes().map(u32::from)),
        Case::Nor { mode } => {
            io::fixed(32, RW_SFI_MISC_CTL3, SFI_CH2_TRANS_IDLE);
            io::fixed(32, RW_SFI_MISC_CTL, SFI_REQ_IDLE);
            let mut status = vec![0, 0, 0, SFI_WIP_READY, 0, 0, 0];
            for _ in 0..if mode == 1 { 9 } else { 7 } {
                status.extend([0, 0, SFI_WIP_READY, 0, 0]);
            }
            io::reads(32, RW_SFI_MAC_CTL, status);
            let bytes = if mode == 1 {
                vec![0xc8, 1, 0, 0, 2, 1, 0]
            } else {
                vec![0xc8, 0, 0, 2, 0]
            };
            io::reads(8, RW_SFI_GPRAM_DATA + 1, bytes);
            io::fixed(8, RW_SFI_GPRAM_DATA + 2, 0x60);
            io::fixed(8, RW_SFI_GPRAM_DATA + 3, 0x17);
            if mode == 2 {
                for offset in 0..4096 {
                    io::value(8, 0x10600000 + offset, 0xff);
                }
            } else {
                for (i, byte) in [0x12, 0x34, 0x56, 0x78].into_iter().enumerate() {
                    io::reads(8, 0x10600000 + i as u32, [0xff, byte]);
                }
            }
        }
        Case::Backlight => {
            for register in [ISINKS_CON0, ISINK0_CON0, ISINK1_CON0] {
                io::value(16, register, 0x8081);
            }
        }
        Case::Uart3 => {
            io::reads(16, UART_LSR(UART3_base), [UART_LSR_DR, 0]);
            io::fixed(16, UART_LSR(UART3_base), UART_LSR_THRE);
            io::fixed(16, UART_RBR(UART3_base), 0x5a);
        }
        Case::KeyMatrix => {
            io::fixed(32, GPIO_INPUT_LEVEL0, 0);
            io::value(32, GPIO_DINV0, 0x80000000);
        }
        Case::Charging => io::value(16, CHR_CON0 + 4, 8),
        Case::Sfi { failure } => {
            io::fixed(32, RW_SFI_MISC_CTL3, SFI_CH2_TRANS_IDLE);
            io::fixed(32, RW_SFI_MISC_CTL, SFI_REQ_IDLE);
            io::fixed(
                8,
                RW_SFI_GPRAM_DATA + 1,
                if failure == 1 { 0 } else { 0xc8 },
            );
            io::fixed(8, RW_SFI_GPRAM_DATA + 2, 0x60);
            io::fixed(8, RW_SFI_GPRAM_DATA + 3, 0x17);
            let mut status = vec![0, 0, 0, SFI_WIP_READY, 0, 0, 0];
            for _ in reference::flash::gd25lq64_profile::DEVICE_COMMANDS {
                status.extend([0, 0, SFI_WIP_READY, 0, 0]);
            }
            io::reads(32, RW_SFI_MAC_CTL, status);
            io::value(16, MPLL_CON0, if failure == 2 { 0 } else { 0x1401 });
            if failure == 3 {
                io::value(32, 0x10000800 + 16, 1);
            }
        }
        Case::Pads { package } => io::value(32, 0xa0110000, package),
        Case::Pmu { efuse, .. } => io::value(32, EKD_PMU_EFUSE, efuse),
        Case::Clock32k | Case::Clock32kBusy => {
            io::fixed(
                32,
                0xa01800a0,
                if matches!(case, Case::Clock32kBusy) {
                    1
                } else {
                    0
                },
            );
            io::fixed(32, 0xa01800ac, 65);
        }
        Case::RtcLatch { fresh } => {
            if !fresh {
                io::value(16, RTC_base + 0x50, 0xa357);
                io::value(16, RTC_base + 0x54, 0x67d2);
            }
            for (offset, value) in [
                (0x14, 0),
                (0x18, 0),
                (0x1c, 0),
                (0x20, 1),
                (0x28, 1),
                (0x24, 1),
                (0x2c, 21),
                (0x30, 0),
                (0x34, 0),
                (0x38, 0),
                (0x3c, 1),
                (0x44, 1),
                (0x48, 0),
            ] {
                io::value(16, RTC_base + offset, value);
            }
        }
        Case::RtcBusy => io::fixed(16, RTC_BBPU, EKD_RTC_CBUSY),
        Case::RtcLatchDenied => io::fixed(16, RTC_BBPU, 0),
        Case::Oscillator { revision } => {
            io::value(32, 0x80000000, revision);
            io::value(16, RTC_XOSCCAL, EKD_RTC_OSC_EMBEDDED);
        }
        Case::Emi | Case::EmiCalibrationFailed => {
            for (i, value) in [0x11, 0x10, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
                .into_iter()
                .enumerate()
            {
                io::value(32, 0x700060dc + i as u32 * 4, value);
            }
            io::fixed(
                32,
                EMI_base + 0x318,
                if matches!(case, Case::Emi) { 2 } else { 3 },
            );
            io::reads(32, EMI_base + 0x90, [0x80, 0]);
        }
        Case::Psram => {
            for (address, value) in [(0x100000, 11), (0x300000, 22), (0x6efffc, 33)] {
                io::value(32, address, value);
            }
        }
        _ => {}
    }
}
fn execute(case: Case, legacy: bool) -> (Vec<io::Run>, bool) {
    setup(case);
    let result = catch_unwind(AssertUnwindSafe(|| match case {
        Case::Dma { mode } => {
            let pixels = vec![0u16; 320 * 240];
            let pixels = if mode == 4 {
                &pixels[..pixels.len() - 1]
            } else {
                &pixels[..]
            };
            let region = if mode == 5 {
                [0, 0, 320, 239]
            } else if mode == 6 {
                [10, 20, 30, 40]
            } else {
                [0, 0, 319, 239]
            };
            if legacy {
                let mut power = reference::pmu::Pmu::<Env>::initialize();
                let mut display = block_on(reference::display::Display::<Env>::initialize(
                    &mut power,
                    reference::display::DisplayPins {
                        reset: 38,
                        cs: 39,
                        clock: 47,
                        bus: [46, 48, 49],
                    },
                ))
                .unwrap();
                block_on(display.present(pixels, region)).unwrap();
            } else {
                let mut power = ekd01_board::power::Power::<Env>::initialize();
                let mut display =
                    block_on(ekd01_board::display::Display::<Env>::initialize(&mut power)).unwrap();
                block_on(display.present(pixels, region)).unwrap();
            }
        }
        Case::BootPlatform => {
            if legacy {
                type B = reference::boot::startup::Startup<Env>;
                reference::boot::emi::Emi::<Env>::initialize();
                reference::boot::rtc::RtcBoot::<Env>::initialize_oscillator();
                B::stabilize_32k();
                let _ = reference::mmio::read32(EMI_GENA);
                assert!(B::probe_psram([0x100000, 0x300000, 0x6efffc]));
                B::prepare_runtime(0x70006140);
                reference::boot::sfi::Sfi::<Env>::initialize();
                B::initialize_power();
                B::release_usb_download_power();
                B::latch_power();
            } else {
                unsafe { ekd01_board::boot::Boot::<Env>::initialize_platform() }.unwrap();
            }
        }
        Case::Handoff => {
            if legacy {
                reference::boot::startup::Startup::<Env>::finish_handoff();
            } else {
                ekd01_board::boot::Boot::<Env>::finish_handoff();
            }
        }
        Case::Watchdog => {
            if legacy {
                reference::boot::startup::Startup::<Env>::arm_application_watchdog();
            } else {
                ekd01_board::boot::Boot::<Env>::arm_application_watchdog();
            }
        }
        Case::Cache => {
            if legacy {
                reference::memory::Memory::<Env>::initialize_cache(0x10198765);
            } else {
                ekd01_board::memory::Memory::<Env>::initialize_cache(0x10198765);
            }
        }
        Case::HeapProbe => {
            assert!(if legacy {
                unsafe { reference::memory::Memory::<Env>::probe_heap(0x100000, 0x101000) }
            } else {
                unsafe { ekd01_board::memory::Memory::<Env>::probe_heap(0x100000, 0x101000) }
            });
        }
        Case::PowerOff => {
            let success = if legacy {
                reference::rtc::Rtc::<Env>::new(45).release_power()
            } else {
                ekd01_board::rtc::Rtc::<Env>::new().release_power()
            };
            assert!(success);
        }

        Case::Backlight => {
            if legacy {
                let mut light = reference::backlight::Backlight::initialize();
                for level in [0, 1, 2, 3, 4, 5, 6] {
                    light.set_level(level);
                }
                light.off();
            } else {
                let mut light = ekd01_board::backlight::Backlight::<Env>::initialize();
                for level in [0, 1, 2, 3, 4, 5, 6] {
                    light.set_level(level);
                }
                light.off();
            }
        }
        Case::Panel { .. } => {
            if legacy {
                let mut power = reference::pmu::Pmu::<Env>::initialize();
                block_on(reference::display::Display::<Env>::initialize(
                    &mut power,
                    reference::display::DisplayPins {
                        reset: 38,
                        cs: 39,
                        clock: 47,
                        bus: [46, 48, 49],
                    },
                ))
                .unwrap();
            } else {
                let mut power = ekd01_board::power::Power::<Env>::initialize();
                block_on(ekd01_board::display::Display::<Env>::initialize(&mut power)).unwrap();
            }
        }
        Case::Uart3 => {
            if legacy {
                let mut uart = reference::uart3::Uart3::<Env>::initialize(0, 1);
                reference::uart3::Uart3::<Env>::irq();
                assert_eq!(uart.receive(), Some(0x5a));
                assert!(uart.send(&[0xf5, 1, 2, 0x0d]));
                assert!(!uart.send(&[]));
                assert!(!uart.send(&[0; 16]));
                uart.stop();
            } else {
                let mut uart = ekd01_board::bridge_uart::BridgeUart::<Env>::initialize();
                ekd01_board::bridge_uart::BridgeUart::<Env>::irq();
                assert_eq!(uart.receive(), Some(0x5a));
                assert!(uart.send(&[0xf5, 1, 2, 0x0d]));
                assert!(!uart.send(&[]));
                assert!(!uart.send(&[0; 16]));
                uart.stop();
            }
        }
        Case::KeyMatrix => {
            let samples = if legacy {
                let mut keys = reference::key_matrix::KeyMatrix::initialize(21, 16, 14);
                [0, 1, 2, 3, 4, 5].map(|now| keys.sample(now))
            } else {
                let mut keys = ekd01_board::keys::KeyMatrix::initialize(21, 16, 14);
                [0, 1, 2, 3, 4, 5].map(|now| keys.sample(now))
            };
            assert_eq!(samples[0], (0, false));
            assert_eq!(samples[1], (0, false));
            assert_eq!(samples[2], (3, true));
        }
        Case::Nor { mode } => {
            use embedded_storage::nor_flash::NorFlash;
            fn exercise<F: NorFlash>(mut flash: F, mode: u8)
            where
                F::Error: core::fmt::Debug,
            {
                if mode == 2 {
                    flash.erase(0, 4096).unwrap();
                } else {
                    flash.write(0, &[0x12, 0x34, 0x56, 0x78]).unwrap();
                }
            }
            if legacy {
                let controller = unsafe { reference::flash::FlashController::attach() };
                exercise(
                    reference::flash::FlashRegion::new(
                        &controller,
                        reference::flash::RegionBounds::new(0x600000, 0x601000).unwrap(),
                    ),
                    mode,
                );
            } else {
                let controller = unsafe { mt2502_hal::flash::FlashController::attach() };
                exercise(
                    mt2502_hal::flash::FlashRegion::new(
                        &controller,
                        mt2502_hal::flash::RegionBounds::new(0x600000, 0x601000).unwrap(),
                    ),
                    mode,
                );
            }
        }
        Case::Uart => {
            if legacy {
                reference::boot::startup::Startup::<Env>::initialize_uart();
            } else {
                mt2502_hal::debug_uart::Uart1::initialize(28);
            }
        }
        Case::Power { .. } | Case::Charging => {
            if legacy {
                reference::boot::startup::Startup::<Env>::initialize_power();
            } else {
                ekd01_board::power::BootPower::<Env>::initialize_power();
            }
        }
        Case::Pads { .. } => {
            if legacy {
                reference::boot::startup::Startup::<Env>::initialize_bus_pads();
            } else {
                mt2502_hal::clocks::Clocks::initialize_bus_pads();
            }
        }
        Case::Clocks { usb } => {
            if legacy {
                reference::boot::startup::Startup::<Env>::initialize_clocks(usb);
            } else {
                ekd01_board::boot::Boot::<Env>::initialize_clocks(usb).unwrap();
            }
        }
        Case::Clock32k | Case::Clock32kBusy => {
            if legacy {
                reference::boot::startup::Startup::<Env>::stabilize_32k();
            } else {
                mt2502_hal::clocks::Clocks::stabilize_32k().unwrap();
            }
        }
        Case::Pmu { .. } => {
            if legacy {
                reference::pmu::Pmu::<Env>::initialize();
            } else {
                ekd01_board::power::Power::<Env>::initialize();
            }
        }
        Case::RtcLatch { .. }
        | Case::RtcBusy
        | Case::RtcLatchDenied
        | Case::RtcEmbedded
        | Case::RtcEmbeddedFresh
        | Case::RtcNoClock
        | Case::RtcCorrupt
        | Case::RtcRollover
        | Case::RtcReadyAfterBusy => {
            if legacy {
                reference::boot::startup::Startup::<Env>::latch_power();
            } else {
                ekd01_board::power::BootPower::<Env>::latch_power().unwrap();
            }
        }
        Case::Oscillator { .. } => {
            if legacy {
                reference::boot::rtc::RtcBoot::<Env>::initialize_oscillator();
            } else {
                mt2502_hal::rtc::startup::RtcStartup::initialize_oscillator().unwrap();
            }
        }
        Case::Emi | Case::EmiCalibrationFailed => {
            if legacy {
                reference::boot::emi::Emi::<Env>::initialize();
            } else {
                ekd01_board::memory::initialize_emi::<Env>().unwrap();
            }
        }
        Case::Sfi { .. } => {
            if legacy {
                reference::boot::sfi::Sfi::<Env>::initialize();
            } else {
                unsafe { ekd01_board::memory::initialize_flash::<Env>() }.unwrap();
            }
        }
        Case::Psram => assert!(if legacy {
            reference::boot::startup::Startup::<Env>::probe_psram([0x100000, 0x300000, 0x6efffc])
        } else {
            mt2502_hal::psram::probe_locations([0x100000, 0x300000, 0x6efffc])
        }),
    }));
    (io::trace(), result.is_ok())
}
fn fingerprint(trace: &[io::Run], succeeded: bool) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    let mut feed = |value: u8| {
        hash = (hash ^ value as u64).wrapping_mul(0x100000001b3);
    };
    feed(succeeded as u8);
    for run in trace {
        feed(run.access.write as u8);
        feed(run.access.width);
        for value in run
            .access
            .address
            .to_le_bytes()
            .into_iter()
            .chain(run.access.value.to_le_bytes())
            .chain((run.count as u64).to_le_bytes())
        {
            feed(value);
        }
    }
    format!(
        "{hash:016x} {} {}\n",
        trace.len(),
        trace.iter().map(|r| r.count).sum::<usize>()
    )
}
#[test]
fn captured_register_sequences_match_frozen_reference() {
    for &(name, case) in CASES {
        let (expected, expected_success) = execute(case, true);
        let should_succeed = match case {
            Case::Clock32kBusy
            | Case::RtcBusy
            | Case::RtcLatchDenied
            | Case::RtcNoClock
            | Case::EmiCalibrationFailed => false,
            Case::Sfi { failure } => failure == 0,
            Case::Panel { id } => id != 0,
            Case::Dma { mode } => mode == 0 || mode == 6,
            _ => true,
        };
        assert_eq!(
            expected_success, should_succeed,
            "reference outcome differs from scenario intent: {name}"
        );
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(format!("{name}.txt"));
        assert!(!expected.is_empty(), "empty register oracle: {name}");
        let digest = fingerprint(&expected, expected_success);
        if std::env::var_os("CAPTURE_REGISTER_REFERENCE").is_some() {
            std::fs::write(&path, &digest).unwrap();
        } else {
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                digest,
                "frozen oracle changed: {name}"
            );
        }
        let expected_ticks = io::ticks();
        let (actual, actual_success) = execute(case, false);
        assert_eq!(io::ticks(), expected_ticks, "delay mismatch: {name}");
        assert_eq!(actual_success, expected_success, "{name}");
        if actual != expected {
            let index = actual
                .iter()
                .zip(&expected)
                .position(|(a, b)| a != b)
                .unwrap_or(actual.len().min(expected.len()));
            panic!(
                "{name}: first difference at {index}; actual {:?}, expected {:?}; lengths {}/{}",
                actual.get(index),
                expected.get(index),
                actual.len(),
                expected.len()
            );
        }
    }
}
#[test]
fn uart_golden_is_halfword_ordered_and_keeps_baud_divider_28() {
    let (trace, success) = execute(Case::Uart, false);
    assert!(success);
    let expected = [
        (0x0c, 0x83),
        (0, 28),
        (4, 0),
        (0x0c, 3),
        (8, 0x47),
        (0x10, 3),
        (4, 0),
        (0x24, 0),
    ];
    assert_eq!(trace.len(), expected.len());
    for (run, (offset, value)) in trace.iter().zip(expected) {
        assert_eq!(
            run.access,
            io::Access {
                write: true,
                width: 16,
                address: UART1_base + offset,
                value
            }
        );
        assert_eq!(run.count, 1);
    }
}
impl mt2502_hal::environment::Runtime for Env {
    fn now() -> u32 {
        io::ticks() as u32
    }
    async fn delay_ticks(ticks: u64) {
        io::advance(ticks);
    }
}
impl reference::environment::Runtime for Env {
    fn now() -> u32 {
        io::ticks() as u32
    }
    async fn delay_ticks(ticks: u64) {
        io::advance(ticks);
    }
}
fn block_on<F: core::future::Future>(future: F) -> F::Output {
    let mut future = Box::pin(future);
    let mut cx = core::task::Context::from_waker(core::task::Waker::noop());
    loop {
        if let core::task::Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
    }
}
#[test]
fn masked_halfword_updates_preserve_reserved_bits_and_width() {
    io::reset();
    io::value(16, 0xa0700000, 0xa5a5);
    mt2502_hal::mmio::update16(0xa0700000, 0x00f0, 0x0020);
    let trace = io::trace();
    assert_eq!(trace.len(), 2);
    assert_eq!(
        trace[0].access,
        io::Access {
            write: false,
            width: 16,
            address: 0xa0700000,
            value: 0xa5a5
        }
    );
    assert_eq!(
        trace[1].access,
        io::Access {
            write: true,
            width: 16,
            address: 0xa0700000,
            value: 0xa525
        }
    );
}
#[test]
fn watchdog_restart_keeps_word_width_and_hardware_key() {
    let (trace, success) = execute(Case::Watchdog, false);
    assert!(success);
    let restart = trace
        .iter()
        .filter(|r| r.access.address == WDT_RESTART)
        .collect::<Vec<_>>();
    assert_eq!(restart.len(), 1);
    assert_eq!(
        restart[0].access,
        io::Access {
            write: true,
            width: 32,
            address: WDT_RESTART,
            value: 0x1971
        }
    );
}
#[test]
fn hardware_errors_are_returned_with_specific_causes() {
    use mt2502_hal::{clocks::Clocks, emi::Emi, error::Error, rtc::startup::RtcStartup};
    setup(Case::RtcBusy);
    assert_eq!(RtcStartup::restore_running_clock(), Err(Error::RtcBusy));
    setup(Case::Clock32kBusy);
    assert_eq!(Clocks::stabilize_32k(), Err(Error::Clock32kBusy));
    setup(Case::EmiCalibrationFailed);
    assert_eq!(
        Emi::initialize(
            &ekd01_board::memory::BootRomTimings,
            ekd01_board::memory::EMI_ARBITRATION
        ),
        Err(Error::EmiCalibrationFailed)
    );
}
#[test]
fn rtc_set_validation_and_calendar_read_are_board_independent() {
    use mt2502_hal::rtc::{ClockTime, Rtc};
    io::reset();
    let mut clock = Rtc;
    assert_eq!(
        clock.try_set(ClockTime {
            hour: 24,
            minute: 0
        }),
        Ok(false)
    );
    assert!(io::trace().is_empty());
    assert_eq!(
        clock.try_set(ClockTime {
            hour: 23,
            minute: 59
        }),
        Ok(true)
    );
    let time = clock.read().unwrap();
    assert_eq!((time.hour, time.minute), (23, 59));
    io::reads(16, RTC_TC_MIN, [59, 0, 0, 0]);
    let time = clock.read().unwrap();
    assert_eq!(time.minute, 0);
}

#[test]
fn frozen_oracle_source_integrity() {
    use sha2::{Digest, Sha256};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/register-reference");
    let manifest = std::fs::read_to_string(root.join("snapshot.sha256")).unwrap();
    for line in manifest.lines() {
        let (expected, path) = line.split_once("  ").unwrap();
        let text = std::fs::read_to_string(root.join(path))
            .unwrap()
            .replace("\r\n", "\n");
        assert_eq!(
            format!("{:x}", Sha256::digest(text.as_bytes())),
            expected,
            "reference changed: {path}"
        );
    }
}
