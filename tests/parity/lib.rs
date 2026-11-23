//! Native source-level checks. The NOR backend is substituted; no CPU is emulated.
extern crate alloc;

#[path = "../../src/soc/nor_geometry.rs"]
pub mod nor_geometry;

#[test]
fn nor_program_commands_preserve_header_and_page_boundaries() {
    use nor_geometry::*;
    for offset in 0..NOR_PAGE_BYTES as u32 {
        for length in [0, 1, 127, 128, 129, 252, 256, 257, 4096] {
            let mut address = 0x501000 + offset;
            let mut remaining = length;
            while remaining != 0 {
                let count = program_chunk(address, remaining);
                assert!(count != 0 && count <= remaining && count <= 128);
                assert_eq!(address / 256, (address + count as u32 - 1) / 256);
                assert!(PROGRAM_COMMAND_BYTES + count <= SFI_GPRAM_BYTES);
                // Model the observed 256-byte GPRAM aperture: loading payload
                // must not wrap and replace the command/address word.
                let mut gpram = [0u8; SFI_GPRAM_BYTES];
                let header = [
                    2,
                    (address >> 16) as u8,
                    (address >> 8) as u8,
                    address as u8,
                ];
                for i in 0..PROGRAM_COMMAND_BYTES + count {
                    gpram[i % SFI_GPRAM_BYTES] = if i < 4 { header[i] } else { 255 };
                }
                assert_eq!(&gpram[..4], &header);
                address += count as u32;
                remaining -= count;
            }
        }
    }
}

#[path = "../../src/bluetooth/patch.rs"]
pub mod controller_patch;

#[test]
fn controller_patch_matches_working_c_payload_without_file_header() {
    // Golden values from the original N22 manifest and C embed_bt_patch output.
    assert_eq!(controller_patch::HEADER_BYTES, 30);
    assert_eq!(controller_patch::PAYLOAD.len(), 0x68f0);
    assert_eq!(
        &controller_patch::PAYLOAD[..16],
        &[
            0xff, 0xff, 0xff, 0xff, 0xe4, 0, 0, 0, 0x30, 0xb0, 2, 0, 0x74, 0xa2, 1, 0
        ]
    );
}

#[path = "../../src/crc.rs"]
pub mod crc;
#[path = "../../src/input.rs"]
pub mod input;
#[path = "../../src/preferences.rs"]
pub mod preferences;
#[path = "../../src/protocol.rs"]
pub mod protocol;
#[path = "../../src/storage.rs"]
pub mod storage;
#[path = "../../src/telemetry.rs"]
pub mod telemetry;

pub mod bluetooth;
pub mod motor;
#[cfg(test)]
mod motor_regression;
pub mod ui;
pub mod soc {
    pub mod bridge_uart {
        use std::{cell::RefCell, collections::VecDeque};
        #[derive(Default)]
        struct Wire {
            input: VecDeque<u8>,
            output: Vec<Vec<u8>>,
            ready: bool,
        }
        std::thread_local! { static WIRE: RefCell<Option<Wire>> = const { RefCell::new(None) }; }
        pub fn reset() {
            WIRE.with(|wire| {
                *wire.borrow_mut() = Some(Wire {
                    ready: true,
                    ..Default::default()
                })
            });
        }
        pub fn advance() {
            WIRE.with(|wire| wire.borrow_mut().as_mut().unwrap().ready = true);
        }
        pub fn inject(frame: crate::protocol::CanFrame) {
            let mut bytes = [0; 15];
            let length = frame.encode(&mut bytes).unwrap();
            WIRE.with(|wire| {
                wire.borrow_mut()
                    .as_mut()
                    .unwrap()
                    .input
                    .extend(&bytes[..length])
            });
        }
        pub fn output() -> Vec<Vec<u8>> {
            WIRE.with(|wire| wire.borrow().as_ref().unwrap().output.clone())
        }
        pub struct BridgeUart;
        impl BridgeUart {
            pub fn initialize() -> Self {
                Self
            }
            pub fn receive(&mut self) -> Option<u8> {
                WIRE.with(|wire| {
                    wire.borrow_mut()
                        .as_mut()
                        .and_then(|wire| wire.input.pop_front())
                })
            }
            pub fn send(&mut self, bytes: &[u8]) -> bool {
                WIRE.with(|wire| {
                    let mut wire = wire.borrow_mut();
                    let Some(wire) = wire.as_mut() else {
                        return false;
                    };
                    if !wire.ready {
                        return false;
                    }
                    wire.ready = false;
                    wire.output.push(bytes.to_vec());
                    true
                })
            }
            pub fn errors(&self) -> u32 {
                0
            }
            pub fn stop(&mut self) {}
        }
    }
    pub struct DebugUart;
    impl core::fmt::Write for DebugUart {
        fn write_str(&mut self, _: &str) -> core::fmt::Result {
            Ok(())
        }
    }
    impl DebugUart {
        pub fn log(_: &str) {}
        pub fn hex(_: &str, _: u32) {}
    }
    pub mod flash {
        pub const CLAIM: u32 = 0x500000;
        pub const STORAGE: u32 = 0x501000;
        pub const STORAGE_END: u32 = 0x560000;
        pub const SECTOR_BYTES: usize = 4096;
        #[derive(Debug)]
        pub enum FlashError {
            Range,
            UnknownDevice,
            NeedsErase,
            Protected,
            Verify,
        }
        pub struct Flash {
            pub bytes: Vec<u8>,
            pub writes: usize,
            pub erases: usize,
            pub operations: usize,
            pub cut: Option<usize>,
            pub available: bool,
            pub erased_offsets: Vec<u32>,
        }
        impl Default for Flash {
            fn default() -> Self {
                Self {
                    bytes: vec![255; 0x800000],
                    writes: 0,
                    erases: 0,
                    operations: 0,
                    cut: None,
                    available: true,
                    erased_offsets: Vec::new(),
                }
            }
        }
        impl Flash {
            pub fn available(&self) -> bool {
                self.available
            }
            pub fn read(&self, offset: u32, out: &mut [u8]) -> Result<(), FlashError> {
                let source = self
                    .bytes
                    .get(offset as usize..offset as usize + out.len())
                    .ok_or(FlashError::Range)?;
                out.copy_from_slice(source);
                Ok(())
            }
            fn fault(&mut self) -> bool {
                self.operations += 1;
                self.cut.is_some_and(|cut| self.operations >= cut)
            }
            pub fn program(&mut self, offset: u32, bytes: &[u8]) -> Result<(), FlashError> {
                if !self.available {
                    return Err(FlashError::UnknownDevice);
                }
                if offset < CLAIM || offset as usize + bytes.len() > self.bytes.len() {
                    return Err(FlashError::Range);
                }
                self.writes += 1;
                let fail = self.fault();
                let count = if fail { bytes.len() / 2 } else { bytes.len() };
                for (target, source) in self.bytes[offset as usize..offset as usize + count]
                    .iter_mut()
                    .zip(bytes)
                {
                    if *target & source != *source {
                        return Err(FlashError::NeedsErase);
                    }
                    *target &= source;
                }
                if fail {
                    Err(FlashError::Verify)
                } else {
                    Ok(())
                }
            }
            pub fn erase(&mut self, offset: u32) -> Result<(), FlashError> {
                if !self.available {
                    return Err(FlashError::UnknownDevice);
                }
                if offset < CLAIM || offset & 4095 != 0 || offset as usize + 4096 > self.bytes.len()
                {
                    return Err(FlashError::Range);
                }
                self.erases += 1;
                self.erased_offsets.push(offset);
                let fail = self.fault();
                self.bytes[offset as usize..offset as usize + if fail { 2048 } else { 4096 }]
                    .fill(255);
                if fail {
                    Err(FlashError::Verify)
                } else {
                    Ok(())
                }
            }
        }
    }
}

#[cfg(test)]
mod checks {
    use super::*;
    use preferences::{Preferences, SaveResult, SettingsManager};
    use protocol::{BridgeMessage, BridgeParser, CanFrame};
    use std::{cell::RefCell, rc::Rc};
    use storage::SettingsStore;

    #[test]
    fn crc_and_controller_wire_fixtures() {
        assert_eq!(crc::crc32(b"123456789"), 0xcbf43926);
        let frame = CanFrame::controls(5, 1, true, false, 3).unwrap();
        assert_eq!(frame.data, [0x55, 11, 1, 0, 0, 0, 0, 0]);
        let mut output = [0; 15];
        let size = frame.encode(&mut output).unwrap();
        assert_eq!(
            &output[..size],
            &[0xf5, 8, 0, 0x63, 0x10, 3, 0x55, 11, 1, 0, 0x0d]
        );
        assert_eq!(
            CanFrame::basic_settings(2500, 260, 2074).unwrap().data,
            [0xc4, 9, 0xa0, 1, 0x1a, 8, 0, 0]
        );
        assert!(CanFrame::controls(5, 6, false, true, 0).is_none());
        assert!(CanFrame::basic_settings(5001, 260, 2074).is_none());
    }
    #[test]
    fn bridge_delimiters_resynchronization_and_gap() {
        let frame = CanFrame::new(0x02f83200, &[0xf5, 0x0d, 1, 2, 3, 4, 5, 6]).unwrap();
        let mut output = [0; 15];
        let size = frame.encode(&mut output).unwrap();
        let mut parser = BridgeParser::new();
        let mut received = Vec::new();
        for byte in [0, 0xf5, 0xff, 1, 2]
            .into_iter()
            .chain(output[..size].iter().copied())
        {
            parser.feed(byte, |message| {
                if let BridgeMessage::Can(frame) = message {
                    received.push(frame);
                }
            });
        }
        // A legal service length may deliberately retain a partial packet until its gap expires.
        parser.timeout_partial();
        for byte in &output[..size] {
            parser.feed(*byte, |message| {
                if let BridgeMessage::Can(frame) = message {
                    received.push(frame);
                }
            });
        }
        assert_eq!(received.last(), Some(&frame));
        assert!(parser.malformed > 0);
    }
    #[test]
    fn mcu_keys_do_not_collapse_buffered_transitions() {
        use input::ButtonFilter;
        assert!(matches!(
            protocol::McuMessage::decode(&[7, 0x21]),
            Some(protocol::McuMessage::Key(0x21))
        ));
        assert!(protocol::McuMessage::decode(&[7, 0x21, 0]).is_none());
        assert!(matches!(
            protocol::McuMessage::decode(&[11, 0x55, 0x27]),
            Some(protocol::McuMessage::Voltage(20138))
        ));
        let mut filter = ButtonFilter::new(0);
        assert_eq!(filter.mcu_event(0x21, 1).unwrap().pressed, 1);
        assert_eq!(filter.mcu_event(0x23, 1).unwrap().short, 1);
        assert_eq!(filter.mcu_event(0x11, 2).unwrap().pressed, 2);
    }
    #[test]
    fn private_creation_language_and_power_loss() {
        let bus = Rc::new(RefCell::new(soc::flash::Flash::default()));
        bus.borrow_mut().bytes[0x500000..0x560000].fill(0x53);
        bus.borrow_mut().bytes[0x560000..].fill(0xa7);
        let protected = crc::crc32(&bus.borrow().bytes[0x560000..]);
        let mut manager = SettingsManager::new(SettingsStore::new(bus.clone()));
        let mut prefs = manager.load();
        assert_eq!(bus.borrow().writes, 0);
        prefs.settings.language = 1;
        assert_eq!(manager.save(&prefs).unwrap(), SaveResult::Saved);
        assert!(bus.borrow().erases < 10);
        assert_eq!(crc::crc32(&bus.borrow().bytes[0x560000..]), protected);
        assert!(
            bus.borrow()
                .erased_offsets
                .iter()
                .all(|offset| (0x500000..0x560000).contains(offset))
        );
        let before = (bus.borrow().writes, bus.borrow().erases);
        assert_eq!(manager.save(&prefs).unwrap(), SaveResult::Unchanged);
        assert_eq!((bus.borrow().writes, bus.borrow().erases), before);
        drop(manager);
        let mut rebooted = SettingsManager::new(SettingsStore::new(bus.clone()));
        assert_eq!(rebooted.load().settings.language, 1);
    }
    #[test]
    fn dashboard_theme_menu_save_reboot_and_reset() {
        use input::ButtonEvents;
        use preferences::DashboardTheme;
        use ui::{
            data::Item,
            model::{Actions, Page, UiModel, View},
        };
        let bus = Rc::new(RefCell::new(soc::flash::Flash::default()));
        let mut manager = SettingsManager::new(SettingsStore::new(bus.clone()));
        let mut model = UiModel::new(manager.load());
        assert_eq!(model.preferences.settings.theme, DashboardTheme::Standard);
        model.navigation.view = View::Menu;
        model.enter(Page::Display);
        model.navigation.selected = (0..model.count())
            .find(|index| model.item(*index) == Item::Theme)
            .unwrap();
        let power = ButtonEvents {
            short: 4,
            released: 4,
            ..Default::default()
        };
        model.step(power, 1);
        assert!(model.navigation.editing);
        model.step(
            ButtonEvents {
                pressed: 1,
                ..Default::default()
            },
            2,
        );
        assert_eq!(model.preferences.settings.theme, DashboardTheme::Classic);
        assert!(model.step(power, 3).contains(Actions::SAVE));
        manager.save(&model.preferences).unwrap();
        let mut rebooted = SettingsManager::new(SettingsStore::new(bus.clone()));
        let mut restored = rebooted.load();
        assert_eq!(restored.settings.theme, DashboardTheme::Classic);
        assert_eq!(rebooted.save(&restored).unwrap(), SaveResult::Unchanged);
        restored.settings.theme.toggle();
        rebooted.save(&restored).unwrap();
        assert_eq!(
            SettingsManager::new(SettingsStore::new(bus))
                .load()
                .settings
                .theme,
            DashboardTheme::Standard
        );
        restored.settings.theme = DashboardTheme::Classic;
        restored.factory_reset();
        assert_eq!(restored.settings.theme, DashboardTheme::Standard);
    }

    #[test]
    fn dashboard_theme_migrates_v3_without_losing_preferences() {
        use preferences::DashboardTheme;
        let bus = Rc::new(RefCell::new(soc::flash::Flash::default()));
        // Build a pre-theme record independently of the current serializer.
        let mut bytes = [0u8; 88];
        for (offset, value) in [
            (0, 0x46504b45u32),
            (4, 3),
            (8, 55),
            (12, 4),
            (16, 1),
            (20, 9),
            (24, 7 | (1 << 8) | (1 << 9) | (2 << 10)),
            (32, 1500),
            (36, 765432),
            (40, 1234567),
            (44, 0x21),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        let header_crc = crc::crc32(&bytes[..28]);
        bytes[28..32].copy_from_slice(&header_crc.to_le_bytes());
        let full_crc = crc::crc32(&bytes[..84]);
        bytes[84..88].copy_from_slice(&full_crc.to_le_bytes());
        SettingsStore::new(bus.clone())
            .write_verified("/prefs/a", &bytes)
            .unwrap();
        let mut manager = SettingsManager::new(SettingsStore::new(bus.clone()));
        let mut preferences = manager.load();
        assert_eq!(preferences.settings.theme, DashboardTheme::Standard);
        assert_eq!(preferences.settings.language, 1);
        assert_eq!(preferences.settings.brightness, 4);
        assert_eq!(preferences.settings.power_mode, 2);
        assert_eq!(preferences.settings.levels, 9);
        assert_eq!(preferences.settings.auto_off_minutes, 7);
        assert_eq!(preferences.settings.service_interval_km, 1500);
        assert_eq!(preferences.odometer_m, 1234567);
        assert_eq!(preferences.since_service_m, 765432);
        assert_eq!(preferences.errors[0], 0x21);
        preferences.settings.theme = DashboardTheme::Classic;
        manager.save(&preferences).unwrap();
        assert_eq!(
            SettingsManager::new(SettingsStore::new(bus)).load(),
            preferences
        );
    }

    #[test]
    fn menu_confirm_exit_and_plus_direction() {
        use input::ButtonEvents;
        use ui::{
            data::Item,
            model::{Actions, Page, UiModel, View},
        };
        let mut model = UiModel::new(Preferences::default());
        model.navigation.view = View::Menu;
        model.enter(Page::Display);
        model.navigation.selected = (0..model.count())
            .find(|index| model.item(*index) == Item::Language)
            .unwrap();
        let power = ButtonEvents {
            short: 4,
            released: 4,
            ..ButtonEvents::default()
        };
        model.step(power, 1);
        assert!(model.navigation.editing);
        model.step(
            ButtonEvents {
                pressed: 1,
                ..ButtonEvents::default()
            },
            2,
        );
        assert_eq!(model.preferences.settings.language, 1);
        assert!(model.step(power, 3).contains(Actions::SAVE));
        assert!(!model.navigation.editing);
        model.save_error = true;
        model.enter(Page::Root);
        model.navigation.selected = model.count() - 1;
        assert!(model.step(power, 4).contains(Actions::SAVE));
        assert_eq!(model.navigation.view, View::Dashboard);
        model.enter(Page::Root);
        model.navigation.view = View::Menu;
        model.navigation.selected = 1;
        model.step(
            ButtonEvents {
                pressed: 1,
                ..ButtonEvents::default()
            },
            5,
        );
        assert_eq!(model.navigation.selected, 0);
    }

    fn legacy_fixture(flash: &mut soc::flash::Flash) {
        // Independent v1 record from the verified C port's migration fixture.
        const RECORD: [u8; 32] = [
            0x45, 0x4b, 0x50, 0x46, 1, 0, 0, 0, 42, 0, 0, 0, 4, 0, 0, 0, 1, 0, 0, 0, 3, 0, 0, 0, 7,
            0, 0, 0, 0x38, 0x08, 0xa6, 0x17,
        ];
        flash.bytes[0x7fe000..0x7fe020].copy_from_slice(&RECORD);
    }
    #[test]
    fn legacy_migration_survives_each_creation_interruption() {
        let baseline = Rc::new(RefCell::new(soc::flash::Flash::default()));
        legacy_fixture(&mut baseline.borrow_mut());
        let mut manager = SettingsManager::new(SettingsStore::new(baseline.clone()));
        let mut prefs = manager.load();
        assert_eq!(prefs.settings.brightness, 4);
        assert!(prefs.settings.imperial && prefs.settings.bluetooth);
        prefs.settings.language = 1;
        manager.save(&prefs).unwrap();
        let operations = baseline.borrow().operations;
        assert!(operations > 3 && operations < 64);
        for cut in 1..=operations {
            let bus = Rc::new(RefCell::new(soc::flash::Flash::default()));
            legacy_fixture(&mut bus.borrow_mut());
            bus.borrow_mut().cut = Some(cut);
            let mut manager = SettingsManager::new(SettingsStore::new(bus.clone()));
            assert_eq!(manager.load().settings.brightness, 4);
            assert!(manager.save(&prefs).is_err());
            drop(manager);
            bus.borrow_mut().cut = None;
            let mut reboot = SettingsManager::new(SettingsStore::new(bus.clone()));
            assert_eq!(reboot.load().settings.brightness, 4, "creation cut {cut}");
            reboot.save(&prefs).unwrap();
            drop(reboot);
            let mut reboot = SettingsManager::new(SettingsStore::new(bus.clone()));
            assert_eq!(reboot.load().settings.language, 1, "recovery cut {cut}");
            assert_eq!(bus.borrow().bytes[0x7fe004], 1);
        }
    }
    #[test]
    fn atomic_replacement_survives_each_modifying_operation() {
        let baseline = Rc::new(RefCell::new(soc::flash::Flash::default()));
        let mut manager = SettingsManager::new(SettingsStore::new(baseline.clone()));
        let mut prefs = Preferences::default();
        prefs.settings.brightness = 4;
        manager.save(&prefs).unwrap();
        let snapshot = baseline.borrow().bytes.clone();
        let before = baseline.borrow().operations;
        prefs.settings.language = 1;
        manager.save(&prefs).unwrap();
        let operations = baseline.borrow().operations - before;
        assert!(operations > 0 && operations < 64);
        for cut in 1..=operations {
            let mut flash = soc::flash::Flash::default();
            flash.bytes.copy_from_slice(&snapshot);
            flash.cut = Some(cut);
            let bus = Rc::new(RefCell::new(flash));
            let mut manager = SettingsManager::new(SettingsStore::new(bus.clone()));
            assert_eq!(manager.load().settings.brightness, 4);
            assert!(manager.save(&prefs).is_err());
            drop(manager);
            bus.borrow_mut().cut = None;
            let mut reboot = SettingsManager::new(SettingsStore::new(bus.clone()));
            let restored = reboot.load();
            assert_eq!(restored.settings.brightness, 4, "replacement cut {cut}");
            assert!(restored.settings.language <= 1);
            reboot.save(&prefs).unwrap();
            drop(reboot);
            assert_eq!(
                SettingsManager::new(SettingsStore::new(bus.clone()))
                    .load()
                    .settings
                    .language,
                1
            );
        }
    }
    #[test]
    fn repeated_saves_relocate_metadata_and_do_not_erase_per_save() {
        let bus = Rc::new(RefCell::new(soc::flash::Flash::default()));
        let mut manager = SettingsManager::new(SettingsStore::new(bus.clone()));
        let mut prefs = Preferences::default();
        manager.save(&prefs).unwrap();
        let initial = bus.borrow().erases;
        for _ in 0..3000 {
            prefs.odometer_m += 10;
            manager.save(&prefs).unwrap();
        }
        assert!(bus.borrow().erases - initial < 1500);
        let mut erased = bus.borrow().erased_offsets.clone();
        erased.sort();
        erased.dedup();
        assert!(
            erased.len() > 3,
            "wear relocation remained confined to the first metadata pair"
        );
        drop(manager);
        assert_eq!(
            SettingsManager::new(SettingsStore::new(bus))
                .load()
                .odometer_m,
            prefs.odometer_m
        );
    }

    fn complete_block_read(
        transfer: &mut motor::transfer::BlockTransfer,
        id: motor::settings::BlockId,
        block: &motor::settings::ParameterBlock,
        now: u32,
    ) {
        assert_eq!(transfer.pending().unwrap().id, 0x03110000 | id as u32);
        transfer.sent(now);
        assert!(transfer.receive(&CanFrame::new(0x02fc0000 | id as u32, &[64]).unwrap(), now));
        for (sequence, chunk) in block.bytes.chunks_exact(8).enumerate() {
            assert_eq!(transfer.pending().unwrap().id, 0x03120000 | id as u32);
            transfer.sent(now);
            let prefix = if sequence == 7 {
                0x02fe0000
            } else {
                0x02fd0000
            };
            assert!(transfer.receive(
                &CanFrame::new(prefix | sequence as u32, chunk).unwrap(),
                now
            ));
        }
        transfer.sent(now);
    }
    #[test]
    fn motor_blocks_write_stop_and_wait_and_verify_all_64_bytes() {
        use motor::{
            settings::{BlockId, Parameter, ParameterBlock},
            transfer::{BlockTransfer, TransferStatus},
        };
        for id in [BlockId::AssistLimits, BlockId::StartResponse] {
            let mut original = ParameterBlock::default();
            for (index, byte) in original.bytes.iter_mut().enumerate() {
                *byte = (index as u8).wrapping_mul(3) ^ 0x5a;
            }
            if id == BlockId::AssistLimits {
                original.bytes[63] = original.checksum();
            }
            let mut transfer = BlockTransfer::default();
            transfer.read(id, 0);
            complete_block_read(&mut transfer, id, &original, 1);
            assert_eq!(transfer.view.state, TransferStatus::Ready);
            let mut draft = original;
            let parameter = if id == BlockId::AssistLimits {
                Parameter::Current
            } else {
                Parameter::Start
            };
            assert!(draft.set(
                parameter,
                5,
                1,
                if parameter == Parameter::Start { 4 } else { 50 }
            ));
            assert!(!transfer.apply(id, &draft, 0, 2));
            assert!(transfer.apply(id, &draft, 1, 2));
            assert_eq!(transfer.pending().unwrap().id, 0x03100000 | id as u32);
            transfer.sent(2);
            let expected = original.prepare(id, &draft).unwrap();
            for index in 0..8 {
                assert!(transfer.receive(&CanFrame::new(0x02fa0000 | id as u32, &[]).unwrap(), 3));
                let frame = transfer.pending().unwrap();
                assert_eq!(frame.length, 8);
                assert_eq!(frame.data, expected.bytes[index * 8..index * 8 + 8]);
                transfer.sent(3);
            }
            assert!(transfer.receive(&CanFrame::new(0x02fa0000 | id as u32, &[]).unwrap(), 4));
            assert_eq!(transfer.view.state, TransferStatus::Verifying);
            complete_block_read(&mut transfer, id, &expected, 5);
            assert_eq!(transfer.view.state, TransferStatus::Verified);
        }
    }
    #[test]
    fn motor_transfer_ignores_bad_sequence_and_shutdown_retry_is_explicit() {
        use motor::{
            settings::BlockId,
            transfer::{BlockTransfer, TransferStatus},
        };
        let mut transfer = BlockTransfer::default();
        transfer.read(BlockId::AssistLimits, 0);
        transfer.sent(0);
        transfer.receive(&CanFrame::new(0x02fc6011, &[64]).unwrap(), 1);
        transfer.sent(1);
        assert!(!transfer.receive(&CanFrame::new(0x02fd0001, &[0; 8]).unwrap(), 2));
        assert_eq!(transfer.view.state, TransferStatus::Reading);
        transfer.cancel();
        transfer.read(BlockId::StartResponse, 3);
        transfer.poll(259);
        assert_eq!(transfer.view.state, TransferStatus::Timeout);
        use ui::power_flow::{PowerFlow, PowerState};
        let mut power = PowerFlow::default();
        power.request(1, true);
        assert!(!power.links_ready(300, true, true));
        power.released(300);
        assert!(!power.links_ready(301, false, false));
        assert!(power.links_ready(428, false, false));
        power.save_finished(false);
        assert_eq!(power.state, PowerState::SaveFailed);
        assert!(power.retry(429, true));
    }
    #[test]
    fn desktop_ble_v1_uses_the_original_three_page_byte_layout() {
        use bluetooth::telemetry::TelemetryService;
        let mut status = motor::client::BikeStatus::default();
        status.active = true;
        status.rx = 1;
        status.levels = 5;
        status.assist = 3;
        let t = &mut status.telemetry;
        t.valid = 3;
        t.speed = 1234;
        t.voltage = 4800;
        t.current = 500;
        t.cadence = 90;
        t.battery = 80;
        t.trip_m = 123450;
        t.range = 2000;
        t.controller_c = 40;
        t.motor_c = 50;
        let bytes = TelemetryService::encode(&status, 10, 0x1234);
        assert_eq!(
            &bytes[..20],
            &[
                1, 1, 0x34, 0x12, 10, 0, 0, 0, 0xd2, 4, 0xc0, 0x12, 0xf4, 1, 0xf0, 0, 90, 80, 3, 43
            ]
        );
        assert_eq!(&bytes[20..28], &[1, 2, 0x34, 0x12, 0x3a, 0xe2, 1, 0]);
        assert_eq!(&bytes[40..44], &[1, 3, 0x34, 0x12]);
        status.telemetry.voltage = 65535;
        status.telemetry.current = 32767;
        assert_eq!(
            &TelemetryService::encode(&status, 10, 0)[14..16],
            &[255, 127]
        );
    }
}
