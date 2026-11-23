//! Distance continuity, checkpoint cadence and versioned trip persistence.
use crate::{
    crc::crc32,
    odometer::{CHECKPOINT_TICKS, DistanceCheckpoint},
    preferences::{Preferences, SaveResult, SettingsManager},
    protocol::CanFrame,
    soc::flash::Flash,
    storage::SettingsStore,
    telemetry::Telemetry,
};
use std::{cell::RefCell, rc::Rc};

fn sample(t: &mut Telemetry, counter: u16, tick: u32) {
    let [lo, hi] = counter.to_le_bytes();
    assert!(t.decode(
        &CanFrame::new(0x02f83200, &[80, lo, hi, 60, 0, 0, 0, 0]).unwrap(),
        tick
    ));
}

#[test]
fn odometer_baseline_duplicates_and_trip_reset_preserve_total() {
    let mut t = Telemetry::default();
    t.trip_m = 200;
    sample(&mut t, 1200, 0);
    assert_eq!((t.trip_m, t.travel_m), (200, 0));
    sample(&mut t, 1203, 128);
    sample(&mut t, 1203, 129);
    assert_eq!((t.trip_m, t.travel_m), (230, 30));
    t.reset_trip();
    sample(&mut t, 1204, 256);
    assert_eq!((t.trip_m, t.travel_m), (10, 40));
    let mut zero = Telemetry::default();
    sample(&mut zero, 0, 0);
    sample(&mut zero, 1, 128);
    assert_eq!(zero.travel_m, 10);
}

#[test]
fn odometer_wrap_and_missing_short_bursts_are_counted() {
    let mut t = Telemetry::default();
    sample(&mut t, 65534, 0);
    sample(&mut t, 2, 128);
    assert_eq!(t.travel_m, 40);
    sample(&mut t, 5, 128 * 4);
    assert_eq!(t.travel_m, 70);
    let mut tick_wrap = Telemetry::default();
    sample(&mut tick_wrap, 10, u32::MAX - 128);
    sample(&mut tick_wrap, 11, 0);
    assert_eq!(tick_wrap.travel_m, 10);
}

#[test]
fn odometer_motor_reset_cannot_add_a_large_false_wrap() {
    for previous in [1200, 32768, 50000, 65000] {
        let mut t = Telemetry::default();
        sample(&mut t, previous, 0);
        sample(&mut t, 0, 128);
        assert_eq!(t.travel_m, 0, "reset from {previous}");
        sample(&mut t, 1, 256);
        assert_eq!(t.travel_m, 10, "new baseline after {previous}");
    }
}

#[test]
fn odometer_corrupt_and_out_of_order_samples_do_not_poison_anchor() {
    let mut t = Telemetry::default();
    sample(&mut t, 1000, 0);
    sample(&mut t, 20000, 128);
    assert_eq!(t.travel_m, 0);
    sample(&mut t, 1001, 256);
    assert_eq!(t.travel_m, 10);
    sample(&mut t, 1000, 257);
    sample(&mut t, 1002, 384);
    assert_eq!(t.travel_m, 20);
    // A repeated new counter establishes a restart without adding the jump.
    sample(&mut t, 0, 512);
    sample(&mut t, 0, 640);
    sample(&mut t, 1, 768);
    assert_eq!(t.travel_m, 30);
}

#[test]
fn odometer_long_outage_rebaselines_and_totals_saturate() {
    let mut t = Telemetry::default();
    sample(&mut t, 50000, 0);
    sample(&mut t, 0, 60 * 128);
    assert_eq!(t.travel_m, 0);
    sample(&mut t, 1, 61 * 128);
    assert_eq!(t.travel_m, 10);
    t.trip_m = u32::MAX - 5;
    t.travel_m = u32::MAX - 5;
    sample(&mut t, 2, 62 * 128);
    assert_eq!((t.trip_m, t.travel_m), (u32::MAX, u32::MAX));
    let mut preferences = Preferences::default();
    preferences.odometer_m = u32::MAX - 5;
    preferences.since_service_m = u32::MAX;
    preferences.record_distance(10);
    assert_eq!(preferences.odometer_m, u32::MAX);
    assert_eq!(preferences.since_service_m, u32::MAX);
}

#[test]
fn odometer_checkpoint_requires_changes_and_waits_60_seconds_after_failure() {
    let mut prefs = Preferences::default();
    let mut checkpoint = DistanceCheckpoint::new(0, &prefs);
    assert!(!checkpoint.due(CHECKPOINT_TICKS, &prefs));
    prefs.record_distance(10);
    prefs.trip_m = 10;
    assert!(!checkpoint.due(CHECKPOINT_TICKS - 1, &prefs));
    assert!(checkpoint.due(CHECKPOINT_TICKS, &prefs));
    checkpoint.finished(CHECKPOINT_TICKS, &prefs, false);
    assert!(!checkpoint.due(2 * CHECKPOINT_TICKS - 1, &prefs));
    assert!(checkpoint.due(2 * CHECKPOINT_TICKS, &prefs));
    checkpoint.finished(2 * CHECKPOINT_TICKS, &prefs, true);
    assert!(!checkpoint.due(3 * CHECKPOINT_TICKS, &prefs));
    prefs.trip_m = 0;
    assert!(checkpoint.due(3 * CHECKPOINT_TICKS, &prefs));
    // An explicit trip reset save establishes a new checkpoint immediately.
    checkpoint.finished(3 * CHECKPOINT_TICKS, &prefs, true);
    assert!(!checkpoint.due(4 * CHECKPOINT_TICKS, &prefs));
    let start = u32::MAX - 100;
    let wrap = DistanceCheckpoint::new(start, &prefs);
    prefs.record_distance(10);
    assert!(!wrap.due(start.wrapping_add(CHECKPOINT_TICKS - 1), &prefs));
    assert!(wrap.due(start.wrapping_add(CHECKPOINT_TICKS), &prefs));
}

#[test]
fn odometer_v4_migrates_to_trip_record_without_losing_other_settings() {
    let bus = Rc::new(RefCell::new(Flash::default()));
    let mut record = [0u8; 88];
    for (offset, value) in [
        (0, 0x46504b45u32),
        (4, 4),
        (8, 41),
        (12, 4),
        (16, 1),
        (20, 9),
        (24, (1 << 8) | (1 << 9) | (1 << 12)),
        (32, 1500),
        (36, 765432),
        (40, 1234567),
        (44, 0x21),
    ] {
        record[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    let header_crc = crc32(&record[..28]);
    record[28..32].copy_from_slice(&header_crc.to_le_bytes());
    let full_crc = crc32(&record[..84]);
    record[84..88].copy_from_slice(&full_crc.to_le_bytes());
    SettingsStore::new(bus.clone())
        .write_verified("/prefs/a", &record)
        .unwrap();
    let mut manager = SettingsManager::new(SettingsStore::new(bus.clone()));
    let old = manager.load();
    assert_eq!(old.trip_m, 0);
    assert_eq!(old.odometer_m, 1234567);
    assert_eq!(old.since_service_m, 765432);
    let mut next = old;
    next.trip_m = 12340;
    manager.save(&next).unwrap();
    let bytes = SettingsStore::new(bus.clone()).read("/prefs/b").unwrap();
    assert_eq!(bytes.len(), 92);
    assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 5);
    let mut reboot = SettingsManager::new(SettingsStore::new(bus.clone()));
    assert_eq!(reboot.load(), next);
    assert!(matches!(reboot.save(&next).unwrap(), SaveResult::Unchanged));
    // Trip shares the full record CRC: damaged trip bytes must not be accepted.
    let mut corrupt = bytes;
    corrupt[84] ^= 1;
    SettingsStore::new(bus.clone())
        .write_verified("/prefs/b", &corrupt)
        .unwrap();
    assert_eq!(SettingsManager::new(SettingsStore::new(bus)).load(), old);
}

#[test]
fn odometer_trip_restore_does_not_recount_it_into_session_distance() {
    use crate::motor::client::MotorClient;
    let mut client = MotorClient::start(0);
    client.restore_trip(12340);
    assert_eq!(client.status().telemetry.trip_m, 12340);
    assert_eq!(client.status().telemetry.travel_m, 0);
    client.clear_trip();
    assert_eq!(client.status().telemetry.trip_m, 0);
    assert_eq!(client.status().telemetry.travel_m, 0);
    let mut model = crate::ui::model::UiModel::new(Preferences::default());
    model.navigation.view = crate::ui::model::View::Menu;
    model.enter(crate::ui::model::Page::Display);
    model.navigation.selected = (0..model.count())
        .find(|row| model.item(*row) == crate::ui::data::Item::Trip)
        .unwrap();
    model.navigation.editing = true;
    model.navigation.confirm_reset = true;
    let actions = model.step(
        crate::input::ButtonEvents {
            short: 4,
            ..Default::default()
        },
        1,
    );
    assert!(
        actions.contains(crate::ui::model::Actions::RESET_TRIP | crate::ui::model::Actions::SAVE)
    );
}
