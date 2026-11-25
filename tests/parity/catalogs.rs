//! Native gettext/runtime checks, without CPU emulation or hardware drivers.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn keys(file: &rspolib::POFile) -> BTreeMap<&str, &rspolib::POEntry> {
    let mut keys = BTreeMap::new();
    for entry in &file.entries {
        if entry.msgid.is_empty() {
            continue;
        }
        assert!(
            !entry.obsolete && entry.msgctxt.is_none(),
            "Unsupported context/obsolete entry"
        );
        assert!(
            keys.insert(entry.msgid.as_str(), entry).is_none(),
            "Duplicate message {}",
            entry.msgid
        );
    }
    keys
}

#[test]
fn catalogs_cover_every_source_message_and_about_label() {
    let root = root();
    let template = rspolib::pofile(root.join("translations/ekd01rust.pot").as_path()).unwrap();
    let expected = keys(&template);
    let mut literals = String::new();
    for entry in fs::read_dir(root.join("ui")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|p| p.to_str()) != Some("slint") {
            continue;
        }
        let text = fs::read_to_string(path).unwrap();
        assert!(
            !text.contains("russian"),
            "UI must use catalogs, not a language boolean"
        );
        let mut position = 0;
        while let Some(offset) = text[position..].find("@tr(\"") {
            let start = position + offset + 4;
            let mut escaped = false;
            let end = text[start + 1..]
                .char_indices()
                .find_map(|(i, c)| {
                    if escaped {
                        escaped = false;
                        return None;
                    }
                    if c == '\\' {
                        escaped = true;
                        None
                    } else {
                        (c == '"').then_some(start + 1 + i + 1)
                    }
                })
                .unwrap();
            let literal = &text[start..end];
            literals += &format!("msgid {literal}\nmsgstr \"\"\n\n");
            position = end;
        }
    }
    let extracted = rspolib::pofile(literals.as_str()).unwrap();
    let actual: BTreeSet<_> = extracted.entries.iter().map(|e| e.msgid.as_str()).collect();
    assert_eq!(
        actual,
        expected.keys().copied().collect(),
        "Refresh the POT after changing @tr strings"
    );
    for locale in crate::localization::LANGUAGES {
        if locale.code == "en" {
            continue;
        }
        let catalog = rspolib::pofile(
            root.join(format!(
                "translations/{}/LC_MESSAGES/ekd01rust.po",
                locale.code
            ))
            .as_path(),
        )
        .unwrap();
        assert_eq!(
            catalog.metadata.get("Language").map(String::as_str),
            Some(locale.code)
        );
        let translated = keys(&catalog);
        assert_eq!(
            translated["English"].msgstr.as_deref(),
            Some(locale.name),
            "The native language name must match the registry and enter the font atlas"
        );
        assert_eq!(
            translated.keys().collect::<Vec<_>>(),
            expected.keys().collect::<Vec<_>>()
        );
        for entry in translated.values() {
            assert!(!entry.flags.iter().any(|f| f == "fuzzy"));
            assert!(
                !entry.msgstr.as_deref().unwrap_or("").is_empty(),
                "Untranslated: {}",
                entry.msgid
            );
        }
        for id in ["About", "Build ID", "Commit", "Unknown", "+ modified"] {
            assert_ne!(
                translated[id].msgstr.as_deref(),
                Some(id),
                "About must be translated"
            );
        }
    }
}

slint::include_modules!();

struct Platform;
impl slint::platform::Platform for Platform {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(
            slint::platform::software_renderer::MinimalSoftwareWindow::new(
                slint::platform::software_renderer::RepaintBufferType::NewBuffer,
            ),
        )
    }
}

#[test]
fn existing_component_updates_when_bundled_language_changes() {
    slint::platform::set_platform(Box::new(Platform)).unwrap();
    let probe = TranslationProbe::new().unwrap();
    slint::select_bundled_translation("en").unwrap();
    assert_eq!(probe.get_label(), "About");
    assert_eq!(probe.get_message(), "Settings save failed");
    assert_eq!(probe.get_formatted_value(), "742 W");
    assert_eq!(probe.get_metric_unit(), "km/h");
    assert_eq!(probe.get_build_label(), "Build ID");
    assert_eq!(probe.get_commit_label(), "Commit");
    assert_eq!(probe.get_unknown_label(), "Unknown");
    assert_eq!(probe.get_modified_label(), "+ modified");
    assert_eq!(probe.get_language_name(), "English");
    slint::select_bundled_translation("ru").unwrap();
    assert_eq!(probe.get_label(), "О программе");
    assert_eq!(probe.get_message(), "Ошибка сохранения");
    assert_eq!(probe.get_formatted_value(), "742 Вт");
    assert_eq!(probe.get_metric_unit(), "км/ч");
    assert_eq!(probe.get_build_label(), "ID сборки");
    assert_eq!(probe.get_commit_label(), "Коммит");
    assert_eq!(probe.get_unknown_label(), "Неизвестно");
    assert_eq!(probe.get_modified_label(), "+ изменения");
    assert_eq!(probe.get_language_name(), "Русский");
    probe.set_item(MenuItem::Language);
    assert_eq!(probe.get_label(), "Язык");
    probe.set_value(DisplayValue {
        kind: ValueKind::Millimetres,
        whole: 2074,
        available: true,
        ..Default::default()
    });
    assert_eq!(probe.get_formatted_value(), "2074 мм");
    probe.set_value(DisplayValue {
        available: false,
        ..probe.get_value()
    });
    assert_eq!(probe.get_formatted_value(), "-- мм");
    slint::select_bundled_translation("es").unwrap();
    assert_eq!(probe.get_label(), "Idioma");
    assert_eq!(probe.get_message(), "Fallo al guardar ajustes");
    assert_eq!(probe.get_build_label(), "ID de compilación");
    assert_eq!(probe.get_commit_label(), "Revisión");
    assert_eq!(probe.get_unknown_label(), "Desconocida");
    assert_eq!(probe.get_modified_label(), "+ modificada");
    assert_eq!(probe.get_language_name(), "Español");
    assert_eq!(probe.get_formatted_value(), "-- mm");
    probe.set_item(MenuItem::About);
    assert_eq!(probe.get_label(), "Acerca de");
    assert!(slint::select_bundled_translation("unavailable").is_err());
    slint::select_bundled_translation("en").unwrap();
    assert_eq!(probe.get_label(), "About");
    assert_eq!(probe.get_formatted_value(), "-- mm");
}

#[test]
fn language_selector_wraps_and_unknown_ids_fall_back_without_rewriting_storage() {
    use crate::localization::{LANGUAGES, Language};
    for (index, language) in LANGUAGES.iter().enumerate() {
        assert_eq!(Language::from_id(language.id), language);
        assert_eq!(
            Language::next(language.id, 1),
            LANGUAGES[(index + 1) % LANGUAGES.len()].id
        );
        assert_eq!(
            Language::next(language.id, -1),
            LANGUAGES[(index + LANGUAGES.len() - 1) % LANGUAGES.len()].id
        );
    }
    assert_eq!(Language::from_id(255).code, "en");
}

#[test]
fn legacy_language_records_migrate_without_losing_other_preferences() {
    use crate::{
        crc::crc32,
        preferences::{DashboardTheme, SaveResult, SettingsManager},
        storage::SettingsStore,
    };
    use std::cell::RefCell;
    for version in 1u32..=5 {
        for language in [0u32, 1] {
            let bus = Rc::new(RefCell::new(crate::soc::flash::Flash::default()));
            let mut bytes = vec![0; if version >= 5 { 92 } else { 88 }];
            let flags = 7
                | if version >= 2 { 1 << 8 } else { 0 }
                | if version >= 3 {
                    (language << 9) | (2 << 10)
                } else {
                    0
                }
                | if version >= 4 { 1 << 12 } else { 0 };
            for (offset, value) in [
                (0, 0x46504b45u32),
                (4, version),
                (8, 7),
                (12, 4),
                (16, 1),
                (20, 9),
                (24, flags),
                (32, 1500),
                (36, 765432),
                (40, 1234567),
                (44, 0x21),
            ] {
                bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            }
            if version >= 5 {
                bytes[84..88].copy_from_slice(&654321u32.to_le_bytes());
            }
            let header = crc32(&bytes[..28]);
            bytes[28..32].copy_from_slice(&header.to_le_bytes());
            if version >= 3 {
                let offset = if version >= 5 { 88 } else { 84 };
                let crc = crc32(&bytes[..offset]);
                bytes[offset..offset + 4].copy_from_slice(&crc.to_le_bytes());
            }
            SettingsStore::new(bus.clone())
                .write_verified("/prefs/a", &bytes)
                .unwrap();
            let mut manager = SettingsManager::new(SettingsStore::new(bus.clone()));
            let preferences = manager.load();
            assert_eq!(
                preferences.settings.language,
                if version >= 3 { language as u8 } else { 0 }
            );
            assert_eq!(preferences.settings.brightness, 4);
            assert!(preferences.settings.imperial);
            assert_eq!(preferences.settings.levels, 9);
            assert_eq!(preferences.settings.auto_off_minutes, 7);
            assert!(preferences.settings.bluetooth);
            assert_eq!(
                preferences.settings.power_mode,
                if version >= 3 { 2 } else { 0 }
            );
            assert_eq!(
                preferences.settings.theme,
                if version >= 4 {
                    DashboardTheme::Classic
                } else {
                    DashboardTheme::Standard
                }
            );
            assert_eq!(
                preferences.odometer_m,
                if version >= 3 { 1234567 } else { 0 }
            );
            assert_eq!(preferences.trip_m, if version >= 5 { 654321 } else { 0 });
            assert_eq!(manager.save(&preferences).unwrap(), SaveResult::Saved);
            assert_eq!(manager.save(&preferences).unwrap(), SaveResult::Unchanged);
            assert_eq!(
                SettingsManager::new(SettingsStore::new(bus.clone())).load(),
                preferences
            );
            let migrated = SettingsStore::new(bus).read("/prefs/b").unwrap();
            assert_eq!(u32::from_le_bytes(migrated[4..8].try_into().unwrap()), 6);
        }
    }
}

#[test]
fn full_language_byte_round_trips_without_overlapping_motor_flags() {
    use crate::{
        preferences::{DashboardTheme, Preferences, SettingsManager},
        storage::SettingsStore,
    };
    use std::cell::RefCell;
    for id in [0, 1, 2, 42, 255] {
        let bus = Rc::new(RefCell::new(crate::soc::flash::Flash::default()));
        let mut preferences = Preferences::default();
        preferences.settings.language = id;
        preferences.settings.theme = DashboardTheme::Classic;
        preferences.settings.power_mode = 3;
        preferences.trip_m = 456789;
        preferences.odometer_m = 1234567;
        SettingsManager::new(SettingsStore::new(bus.clone()))
            .save(&preferences)
            .unwrap();
        assert_eq!(
            SettingsManager::new(SettingsStore::new(bus)).load(),
            preferences
        );
    }
}
