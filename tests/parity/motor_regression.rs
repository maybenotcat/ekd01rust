//! Source-level regressions for N22 routing and the automatic settings flow.
use crate::{
    input::ButtonEvents,
    motor::{
        client::{BasicStatus, MotorClient},
        settings::{BlockId, Parameter, ParameterBlock},
        transfer::{AdvancedView, BlockTransfer, RequestResult, TransferStatus},
    },
    preferences::Preferences,
    protocol::{BridgeMessage, BridgeParser, CanFrame},
    soc::bridge_uart as wire,
    ui::{
        data::Item,
        model::{Actions, Page, UiModel, View},
    },
};

fn reply(destination: u32, operation: u32, command: u16, data: &[u8]) -> CanFrame {
    CanFrame::new(
        0x02000000 | (destination << 19) | (operation << 16) | command as u32,
        data,
    )
    .unwrap()
}

fn receive_block(
    transfer: &mut BlockTransfer,
    id: BlockId,
    destination: u32,
    block: &ParameterBlock,
) {
    assert_eq!(transfer.pending().unwrap().id, 0x03110000 | id as u32);
    transfer.sent(1);
    assert!(!transfer.receive(&CanFrame::new(0x011c0000 | id as u32, &[64]).unwrap(), 1));
    assert!(transfer.receive(&reply(destination, 4, id as u16, &[64]), 1));
    for (sequence, bytes) in block.bytes.chunks_exact(8).enumerate() {
        assert_eq!(transfer.pending().unwrap().id, 0x03120000 | id as u32);
        transfer.sent(1);
        assert!(transfer.receive(
            &reply(
                destination,
                if sequence == 7 { 6 } else { 5 },
                sequence as u16,
                bytes
            ),
            1
        ));
    }
    transfer.sent(1);
}

#[test]
fn motor_settings_directed_read_write_and_full_verification() {
    // N22 100CDC70 checks source & 0x1f, 100EB9E4 checks operation & 7.
    // Both directed and broadcast destinations must reach the same state machine.
    for destination in [3, 5, 31] {
        for id in [BlockId::AssistLimits, BlockId::StartResponse] {
            let mut original = ParameterBlock::default();
            original.bytes.fill(17);
            original.bytes[63] = original.checksum();
            let mut transfer = BlockTransfer::default();
            transfer.read(id, 0);
            receive_block(&mut transfer, id, destination, &original);
            assert_eq!(transfer.view.state, TransferStatus::Ready);
            let parameter = if id == BlockId::AssistLimits {
                Parameter::Current
            } else {
                Parameter::Start
            };
            let mut draft = original;
            assert!(draft.set(parameter, 5, 1, 4));
            let expected = original.prepare(id, &draft).unwrap();
            assert!(transfer.apply(id, &draft, 1, 2));
            transfer.sent(2);
            for index in 0..8 {
                assert!(transfer.receive(&reply(destination, 2, id as u16, &[]), 2));
                let frame = transfer.pending().unwrap();
                assert_eq!(frame.data, expected.bytes[index * 8..index * 8 + 8]);
                transfer.sent(2);
            }
            assert!(transfer.receive(&reply(destination, 2, id as u16, &[]), 2));
            receive_block(&mut transfer, id, destination, &expected);
            assert_eq!(transfer.view.state, TransferStatus::Verified);
            transfer.read(id, 3);
            assert!(
                !(if id == BlockId::AssistLimits {
                    transfer.view.limits
                } else {
                    transfer.view.start
                })
                .valid
            );
        }
    }
}

fn transmitted_ids() -> Vec<u32> {
    let mut ids = Vec::new();
    let mut parser = BridgeParser::new();
    for packet in wire::output() {
        for byte in packet {
            parser.feed(byte, |message| {
                if let BridgeMessage::Can(frame) = message {
                    ids.push(frame.id);
                }
            });
        }
    }
    ids
}

#[test]
fn motor_settings_n22_variable_chunks_and_unchecked_read_checksum() {
    let mut transfer = BlockTransfer::default();
    transfer.read(BlockId::AssistLimits, 0);
    transfer.sent(0);
    // 100EB9E4 consumes the first header byte, without requiring DLC == 1.
    assert!(transfer.receive(&reply(3, 4, 0x6011, &[64, 0]), 1));
    transfer.sent(1);
    assert!(!transfer.receive(&reply(3, 5, 7, &[1; 8]), 2));
    assert_eq!(transfer.view.state, TransferStatus::Reading);
    // Partial data followed by a new matching header restarts the assembly.
    assert!(transfer.receive(&reply(3, 5, 0, &[7; 3]), 3));
    transfer.sent(3);
    assert!(transfer.receive(&reply(3, 4, 0x6011, &[64]), 4));
    transfer.sent(4);
    // 1010DB38/1010D9C4 copy the actual DLC. Neither validates byte 63.
    let original = ParameterBlock { bytes: [5; 64] };
    for (sequence, chunk) in original.bytes.chunks(7).enumerate() {
        let operation = if sequence == 9 { 6 } else { 5 };
        assert!(transfer.receive(&reply(3, operation, sequence as u16, chunk), 5));
        transfer.sent(5);
    }
    assert_eq!(transfer.view.state, TransferStatus::Ready);
    assert_eq!(transfer.view.limits.block, original);
    let mut edited = original;
    assert!(edited.set(Parameter::Current, 5, 1, 25));
    assert!(transfer.apply(BlockId::AssistLimits, &edited, 1, 6));
    let prepared = original.prepare(BlockId::AssistLimits, &edited).unwrap();
    assert_eq!(prepared.bytes[63], prepared.checksum());
}

#[test]
fn motor_settings_original_level_count_and_control_vectors() {
    for levels in [3, 5, 9] {
        assert_eq!(
            CanFrame::assist_levels(levels).unwrap(),
            CanFrame::new(0x05103209, &[levels, 0]).unwrap()
        );
    }
    // 101E527C and 100EBA84: five-level assist 3, power mode 3 maps to 5.
    assert_eq!(
        CanFrame::controls(5, 3, true, true, 3).unwrap(),
        CanFrame::new(0x03106300, &[0x55, 0x15, 1, 1]).unwrap()
    );
    wire::reset();
    let mut client = MotorClient::start(0);
    client.controls(9, 0, false, 0);
    client.service(0, |_| {});
    wire::advance();
    client.service(1, |_| {});
    wire::advance();
    client.service(2, |_| {});
    assert_eq!(
        transmitted_ids()
            .iter()
            .filter(|id| **id == 0x05103209)
            .count(),
        1
    );
    client.controls(9, 1, false, 0);
    wire::advance();
    client.service(3, |_| {});
    assert_eq!(
        transmitted_ids()
            .iter()
            .filter(|id| **id == 0x05103209)
            .count(),
        1
    );
}

#[test]
fn motor_settings_original_3203_uses_wheel_without_trailing_word() {
    let mut telemetry = crate::telemetry::Telemetry::default();
    assert!(telemetry.decode(&reply(3, 0, 0x3203, &[0xc4, 9, 0xb5, 1, 0, 0]), 1));
    assert_eq!(telemetry.limit, 2500);
    assert_eq!(telemetry.wheel, 275);
    assert_eq!(
        telemetry.circumference,
        crate::protocol::wheel_circumference(275).unwrap()
    );
    let mut model = UiModel::new(Preferences::default());
    model.motor_readback(telemetry.limit, telemetry.wheel, telemetry.circumference);
    assert_eq!(model.basic.valid, 7);
    // N22 dispatches operation 7 independently of destination, including errors.
    assert!(telemetry.decode(&reply(3, 7, 0x1300, &[7]), 2));
    assert_ne!(telemetry.error, 0);
}

#[test]
fn motor_settings_received_values_are_not_hidden_by_write_bounds() {
    let mut model = UiModel::new(Preferences::default());
    // The previous UI rejected every field if the limit exceeded 50 km/h or
    // the wheel lay outside 10..29 inches, despite a complete 3203 status.
    for (limit, wheel) in [(6000, 275), (2500, 320), (100, 60), (9900, 340)] {
        let circumference = crate::protocol::wheel_circumference(wheel).unwrap();
        model.motor_readback(limit, wheel, circumference);
        assert_eq!(model.basic.valid, 7);
        assert_eq!(model.basic.limit_x10, limit / 10);
        assert_eq!(model.basic.wheel, wheel);
        assert!(CanFrame::basic_settings(limit, wheel, circumference).is_some());
    }
    // Received data is visible, but invalid values cannot be transmitted.
    model.motor_readback(u16::MAX, 0, 0);
    assert_eq!(model.basic.valid, 7);
    assert_eq!(model.basic.wheel, 0);
    assert!(CanFrame::basic_settings(u16::MAX, 0, 0).is_none());
}

#[test]
fn motor_settings_client_reads_start_before_limits() {
    wire::reset();
    let mut client = MotorClient::start(0);
    assert_eq!(
        client.read_block(BlockId::AssistLimits, 0),
        RequestResult::Accepted
    );
    let mut now = 0;
    for _ in 0..3 {
        wire::advance();
        client.service(now, |_| {});
        now += 1;
    }
    assert!(transmitted_ids().contains(&0x03116010));
    assert!(!transmitted_ids().contains(&0x03116011));
    wire::inject(reply(3, 4, 0x6010, &[64]));
    wire::advance();
    client.service(now, |_| {});
    now += 1;
    for sequence in 0..8 {
        wire::inject(reply(
            3,
            if sequence == 7 { 6 } else { 5 },
            sequence,
            &[5; 8],
        ));
        wire::advance();
        client.service(now, |_| {});
        now += 1;
    }
    assert!(client.busy());
    wire::advance();
    client.service(now, |_| {});
    assert!(client.status().advanced.start.valid);
    assert!(transmitted_ids().contains(&0x03116011));
    assert!(!client.status().advanced.limits.valid);
}

#[test]
fn motor_settings_basic_status_establishes_connection_and_serializes_writes() {
    wire::reset();
    let mut client = MotorClient::start(0);
    assert_eq!(client.read_basic(0), RequestResult::Accepted);
    client.service(0, |_| {});
    assert!(!transmitted_ids().contains(&0x03113203));
    assert_eq!(client.status().basic_state, BasicStatus::Reading);
    wire::inject(reply(3, 0, 0x3203, &[0xc4, 9, 0xb5, 1, 0x92, 8]));
    wire::advance();
    client.service(1, |_| {});
    assert_eq!(client.status().basic_state, BasicStatus::Loaded);
    assert_eq!(client.status().telemetry.limit, 2500);
    assert!(client.status().motor_online(1));
    assert_eq!(
        client.apply_basic([2600, 275, 2194], 2),
        RequestResult::Accepted
    );
    assert_eq!(
        client.read_block(BlockId::AssistLimits, 2),
        RequestResult::Busy
    );
    wire::advance();
    client.service(2, |_| {});
    assert_eq!(client.status().basic_state, BasicStatus::Sent);
    wire::inject(reply(5, 0, 0x3203, &[0xc4, 9, 0xb5, 1, 0x92, 8]));
    wire::advance();
    client.service(3, |_| {});
    assert_eq!(client.status().basic_state, BasicStatus::Sent);
    assert_eq!(transmitted_ids().last(), Some(&0x05103203));
    wire::inject(reply(5, 0, 0x3203, &[0x28, 10, 0xb5, 1, 0x92, 8]));
    wire::advance();
    client.service(4, |_| {});
    assert_eq!(client.status().basic_state, BasicStatus::Verified);
    assert_eq!(
        transmitted_ids()
            .iter()
            .filter(|id| **id == 0x05103203)
            .count(),
        1
    );
    assert_eq!(client.read_basic(5), RequestResult::Accepted);
    assert_eq!(client.status().basic_state, BasicStatus::Loaded);
    assert!(!client.busy());
    assert!(!transmitted_ids().contains(&0x03113203));
}

fn select(model: &mut UiModel, item: Item) {
    model.navigation.selected = (0..model.count())
        .find(|row| model.item(*row) == item)
        .unwrap();
}
fn power(model: &mut UiModel) -> Actions {
    model.step(
        ButtonEvents {
            short: 4,
            ..Default::default()
        },
        1,
    )
}

#[test]
fn motor_settings_menu_reads_on_entry_and_commits_on_edit_completion() {
    let mut model = UiModel::new(Preferences::default());
    model.navigation.view = View::Menu;
    select(&mut model, Item::Motor);
    assert!(power(&mut model).contains(Actions::MOTOR_READ));
    select(&mut model, Item::Limit);
    power(&mut model);
    assert!(!model.navigation.editing);
    model.motor_readback(2500, 275, 2194);
    model.motor_online = true;
    power(&mut model);
    assert!(model.navigation.editing);
    assert!(!power(&mut model).contains(Actions::MOTOR_APPLY));
    power(&mut model);
    model.step(
        ButtonEvents {
            pressed: 1,
            ..Default::default()
        },
        2,
    );
    assert!(power(&mut model).contains(Actions::MOTOR_APPLY));
    model.motor_busy = true;
    select(&mut model, Item::Wheel);
    power(&mut model);
    assert!(!model.navigation.editing);
    select(&mut model, Item::Back);
    power(&mut model);
    assert_eq!(model.navigation.page, Page::Root);

    model.motor_busy = false;
    model.enter(Page::Motor);
    select(&mut model, Item::Advanced);
    assert!(power(&mut model).contains(Actions::ADV_READ));
    let mut view = AdvancedView::default();
    view.limits.valid = true;
    view.limits.revision = 1;
    view.limits.block.bytes.fill(20);
    model.advanced_readback(&view);
    select(&mut model, Item::Current);
    power(&mut model);
    power(&mut model);
    assert!(model.navigation.editing);
    model.step(
        ButtonEvents {
            pressed: 2,
            ..Default::default()
        },
        2,
    );
    assert!(power(&mut model).contains(Actions::ADV_APPLY));
    model.enter(Page::Advanced);
    select(&mut model, Item::Start);
    assert!(power(&mut model).contains(Actions::START_READ));
    power(&mut model);
    assert!(!model.navigation.editing);
}
