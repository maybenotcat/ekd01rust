//! Presentation contracts checked against the previous user-visible strings.
use super::*;

pub fn check(dashboard: &Dashboard) {
    let text = dashboard.global::<UiText>();
    let statuses = [
        (StatusCode::None, "", ""),
        (StatusCode::SaveError, "Save error", "Ошибка"),
        (StatusCode::Edit, "EDIT", "ПРАВКА"),
        (StatusCode::Apply, "Apply", "Записать"),
        (StatusCode::Busy, "Busy", "Занято"),
        (StatusCode::Offline, "CAN offline", "Нет связи CAN"),
        (StatusCode::Neutral, "Set assist 0", "Нужен ассист 0"),
        (StatusCode::Invalid, "Set values", "Нет значений"),
        (StatusCode::Reading, "Reading", "Чтение"),
        (StatusCode::Loaded, "Loaded", "Прочитано"),
        (StatusCode::Queued, "Queued", "В очереди"),
        (StatusCode::Sent, "Sent", "Отправлено"),
        (StatusCode::ReadbackOk, "Readback OK", "Прочитано"),
        (StatusCode::Timeout, "No reply", "Нет ответа"),
        (StatusCode::ReadFirst, "Read first", "Сначала чтение"),
        (StatusCode::Modified, "Modified", "Изменено"),
        (StatusCode::Writing, "Writing", "Запись"),
        (StatusCode::Verifying, "Verifying", "Проверка"),
        (StatusCode::Verified, "Verified", "Совпадает"),
        (StatusCode::BadBlock, "Bad block", "Неверный блок"),
        (StatusCode::Mismatch, "Mismatch", "Не совпадает"),
        (StatusCode::PoweringOff, "Powering off", "Выключение"),
        (
            StatusCode::SettingsSaveFailed,
            "Settings save failed",
            "Ошибка сохранения",
        ),
        (
            StatusCode::PowerControlError,
            "Power control error",
            "Ошибка выключения",
        ),
        (StatusCode::ServiceDue, "SERVICE DUE", "ПОРА ПРОЙТИ ТО"),
    ];
    for (code, english, russian) in statuses {
        assert_eq!(text.invoke_status(code, false), english);
        assert_eq!(text.invoke_status(code, true), russian);
    }

    for (kind, whole, fraction, english, russian) in [
        (ValueKind::Number, 9, 0, "9", "9"),
        (ValueKind::Percent, 78, 0, "78%", "78%"),
        (ValueKind::DistanceHundredths, 12, 3, "12.03 km", "12.03 км"),
        (ValueKind::DistanceTenths, 321, 0, "321.0 km", "321.0 км"),
        (ValueKind::Voltage, 52, 7, "52.7 V", "52.7 V"),
        (ValueKind::Power, -742, 0, "-742 W", "-742 Вт"),
        (ValueKind::MetricSpeed, 25, 0, "25.0 km/h", "25.0 км/ч"),
        (ValueKind::Wheel, 26, 0, "26.0 in", "26.0 in"),
        (ValueKind::Millimetres, 2074, 0, "2074 mm", "2074 мм"),
        (ValueKind::Kilometres, 500, 0, "500 km", "500 км"),
        (ValueKind::Cadence, 72, 0, "72 rpm", "72 об/мин"),
        (ValueKind::Temperature, -40, 0, "-40 C", "-40 C"),
        (ValueKind::Clock, 3, 4, "03:04", "03:04"),
        (ValueKind::Minutes, 5, 0, "5 min", "5 мин"),
        (ValueKind::Minutes, 0, 0, "Off", "Выкл"),
        (ValueKind::Levels, 5, 0, "5 levels", "5 уровней"),
        (ValueKind::Mode, 2, 0, "Mode 2", "Режим 2"),
        (ValueKind::StartResponse, 3, 0, "3 / 8", "3 / 8"),
    ] {
        let value = sample_value(kind, whole, fraction);
        assert_eq!(text.invoke_value(value.clone(), false, false), english);
        assert_eq!(text.invoke_value(value, true, false), russian);
    }
    for (kind, english, russian) in [
        (ValueKind::Power, "-- W", "-- Вт"),
        (ValueKind::Voltage, "--.- V", "--.- V"),
        (ValueKind::Percent, "--%", "--%"),
        (ValueKind::MetricSpeed, "-- km/h", "-- км/ч"),
        (ValueKind::Wheel, "-- in", "-- in"),
        (ValueKind::Millimetres, "-- mm", "-- mm"),
        (ValueKind::Cadence, "-- rpm", "-- об/мин"),
        (ValueKind::Temperature, "-- C", "-- C"),
        (ValueKind::Clock, "--:--", "--:--"),
        (ValueKind::Unavailable, "--", "--"),
    ] {
        let value = DisplayValue {
            kind,
            ..Default::default()
        };
        assert_eq!(text.invoke_value(value.clone(), false, false), english);
        assert_eq!(text.invoke_value(value, true, false), russian);
    }
    for (kind, flag, english, russian) in [
        (ValueKind::Toggle, true, "On", "Вкл"),
        (ValueKind::Toggle, false, "Off", "Выкл"),
        (ValueKind::Confirmation, true, "Yes", "Да"),
        (ValueKind::Confirmation, false, "No", "Нет"),
        (ValueKind::Theme, true, "Classic", "Классическая"),
        (ValueKind::Theme, false, "Standard", "Стандартная"),
        (ValueKind::Language, true, "Русский", "Русский"),
        (ValueKind::Language, false, "English", "English"),
    ] {
        let value = DisplayValue {
            kind,
            flag,
            ..Default::default()
        };
        assert_eq!(text.invoke_value(value.clone(), false, false), english);
        assert_eq!(text.invoke_value(value, true, false), russian);
    }
    assert_eq!(
        text.invoke_value(
            sample_value(ValueKind::DistanceHundredths, 1, 1),
            true,
            true
        ),
        "1.01 ми"
    );
    assert_eq!(text.invoke_speed_unit(true, true), "миль/ч");

    // Counters and hexadecimal fields must not lose bits through float conversion.
    for value in [0, 9, 10_000, 16_777_217, 2_147_483_648, u32::MAX] {
        assert_eq!(text.invoke_counter(counter(value)), value.to_string());
    }
    for code in 0..=255 {
        assert_eq!(text.invoke_hex2(code), format!("{code:02X}"));
    }
    assert_eq!(text.invoke_hex6(0xc86017), "C86017");
    assert_eq!(
        text.invoke_dashboard_error(true, 0x21, true, false),
        "ERROR 21"
    );
    assert_eq!(
        text.invoke_dashboard_error(true, 0x21, true, true),
        "ОШИБКА 21"
    );
    assert_eq!(text.invoke_dashboard_error(false, 0, false, true), "");

    let mut info = InformationData {
        voltage: sample_value(ValueKind::Voltage, 52, 7),
        battery: sample_value(ValueKind::Percent, 78, 0),
        controller_available: true,
        limit: 2500,
        wheel: 260,
        circumference: 2074,
        can_rx: counter(u32::MAX),
        flash_jedec: 0xc86017,
        mcu0: 0x24,
        mcu1: 0x09,
        mcu2: 3,
        mcu3: 0x16,
        mcu4: 0x55,
        hour: 3,
        minute: 4,
        ..Default::default()
    };
    assert_eq!(
        text.invoke_information(MenuItem::BatteryInfo, info.clone(), false),
        "Total voltage  52.7 V\nSOC  78%"
    );
    assert_eq!(
        text.invoke_information(MenuItem::BatteryInfo, info.clone(), true),
        "Напряжение  52.7 V\nSOC  78%"
    );
    assert_eq!(
        text.invoke_information(MenuItem::CtrlInfo, info.clone(), false),
        "Speed Limit  25.00 km/h\nWheel Size  26.0 in\nWheel Length  2074 mm\nCAN RX  4294967295\nSerial / version  --"
    );
    assert_eq!(
        text.invoke_information(MenuItem::DisplayInfo, info.clone(), false),
        "LCD  320x240\nFLASH  C86017 / 8 MiB\nPSRAM  8 MiB\nMCU  [24, 09, 03, 16, 55]\n03:04"
    );
    assert_eq!(
        text.invoke_information(MenuItem::Errors, info.clone(), true),
        "Ошибок в журнале нет"
    );
    info.error_count = 3;
    info.error_position = 2;
    info.error_code = 0x21;
    info.error_source = 2;
    info.error_command = 0x17;
    info.error_subcommand = 3;
    assert_eq!(
        text.invoke_information(MenuItem::Errors, info, false),
        "Error 21\nSource 2 / cmd 17:3\n2 / 3"
    );
    let diagnostics = DiagnosticsData {
        flash_writable: true,
        flash_jedec: 0xc86017,
        can_rx: counter(u32::MAX),
        can_tx: counter(16_777_217),
        mcu_rx: counter(0),
        adc_mv: counter(52700),
        adc_fresh: false,
        last_can_high: 0xffff,
        last_can_low: 0xffff,
        ..Default::default()
    };
    assert_eq!(
        text.invoke_diagnostics(diagnostics, false),
        "PSRAM  OK\nFLASH  OK / C86017\nCAN RX  4294967295  TX  16777217\nMCU RX  0\nADC  52700 mV / stale\nUART errors  0\nBad frames  0\nLast CAN  FFFFFFFF"
    );
    println!(
        "Slint text: EN/RU statuses, units, missing data, counters, faults and information OK"
    );
}

fn counter(value: u32) -> CounterValue {
    CounterValue {
        upper: (value / 10_000) as i32,
        lower: (value % 10_000) as i32,
    }
}
