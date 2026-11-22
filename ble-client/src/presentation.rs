use crate::protocol::Snapshot;

pub const LABELS: [&str; 15] = [
    "Speed",
    "Power",
    "Voltage",
    "Current",
    "Battery",
    "Cadence",
    "Assist request",
    "Lights request",
    "Trip distance",
    "Range",
    "Controller temperature",
    "Motor temperature",
    "CAN RX / TX",
    "UART errors / bad frames",
    "MCU RX",
];
pub fn values(snapshot: &Snapshot) -> Vec<String> {
    let show = |valid: bool, value: String| if valid { value } else { "—".into() };
    let electric = snapshot.electric_fresh();
    let ride = snapshot.ride_fresh();
    vec![
        show(
            electric,
            format!("{:.1} km/h", snapshot.speed as f64 / 100.0),
        ),
        show(electric, format!("{} W", snapshot.power)),
        show(
            electric || snapshot.adc_fresh(),
            format!("{:.2} V", snapshot.voltage as f64 / 100.0),
        ),
        show(
            electric,
            format!("{:.2} A", snapshot.current as f64 / 100.0),
        ),
        show(
            ride && snapshot.battery <= 100,
            format!("{} %", snapshot.battery),
        ),
        show(ride, format!("{} rpm", snapshot.cadence)),
        format!("{} / {}", snapshot.assist, snapshot.levels),
        if snapshot.flags & 16 != 0 {
            "On".into()
        } else {
            "Off".into()
        },
        format!("{:.2} km", snapshot.trip_metres as f64 / 1000.0),
        show(ride, format!("{:.1} km", snapshot.range as f64 / 100.0)),
        show(electric, format!("{} °C", snapshot.controller_c)),
        show(electric, format!("{} °C", snapshot.motor_c)),
        format!("{} / {}", snapshot.can_rx, snapshot.can_tx),
        format!("{} / {}", snapshot.uart_errors, snapshot.malformed),
        format!("{}", snapshot.mcu_rx),
    ]
}
pub fn status(snapshot: &Snapshot) -> String {
    format!(
        "Sequence {} · Voltage: {} · CAN {}",
        snapshot.sequence,
        snapshot.voltage_source(),
        if snapshot.flags & 32 != 0 {
            "active"
        } else {
            "stale"
        }
    )
}
pub fn self_test() -> Result<(), String> {
    let mut snapshot =
        Snapshot::decode(include_bytes!("../fixtures/snapshot-v1.bin")).ok_or("Invalid fixture")?;
    snapshot.flags |= 2;
    let cards = values(&snapshot);
    if cards.len() != LABELS.len()
        || cards[0] != "25.3 km/h"
        || cards[1] != "-120 W"
        || cards[3] != "-2.50 A"
    {
        return Err("Signed telemetry/unit conversion failed".into());
    }
    snapshot.flags = 4;
    let cards = values(&snapshot);
    if cards[0] != "—"
        || cards[1] != "—"
        || cards[2] == "—"
        || cards[4] != "—"
        || snapshot.voltage_source() != "MCU ADC"
    {
        return Err("ADC fallback or freshness masking failed".into());
    }
    Ok(())
}
