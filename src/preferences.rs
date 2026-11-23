use crate::{crc::crc32, soc::DebugUart, storage::SettingsStore};
use littlefs_rust::Error;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum DashboardTheme {
    #[default]
    Standard,
    Classic,
}
impl DashboardTheme {
    pub fn toggle(&mut self) {
        *self = match self {
            Self::Standard => Self::Classic,
            Self::Classic => Self::Standard,
        };
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub theme: DashboardTheme,
    pub brightness: u8,
    pub imperial: bool,
    pub levels: u8,
    pub auto_off_minutes: u8,
    pub bluetooth: bool,
    pub language: u8,
    pub power_mode: u8,
    pub service_interval_km: u32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: DashboardTheme::Standard,
            brightness: 3,
            imperial: false,
            levels: 5,
            auto_off_minutes: 0,
            bluetooth: true,
            language: 0,
            power_mode: 0,
            service_interval_km: 0,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Preferences {
    pub settings: Settings,
    pub since_service_m: u32,
    pub odometer_m: u32,
    pub trip_m: u32,
    pub errors: [u32; 10],
}
impl Preferences {
    pub fn record_distance(&mut self, metres: u32) {
        self.since_service_m = self.since_service_m.saturating_add(metres);
        self.odometer_m = self.odometer_m.saturating_add(metres);
    }
    pub fn record_error(&mut self, error: u32) {
        if error != 0 && self.errors[0] != error {
            self.errors.copy_within(0..9, 1);
            self.errors[0] = error;
        }
    }
    pub fn factory_reset(&mut self) {
        self.settings = Settings::default();
    }
}

#[derive(Clone)]
struct Record {
    bytes: [u8; 92],
}
impl Record {
    fn word(&self, offset: usize) -> u32 {
        u32::from_le_bytes(self.bytes[offset..offset + 4].try_into().unwrap())
    }
    fn put(&mut self, offset: usize, value: u32) {
        self.bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn decode(bytes: &[u8]) -> Option<Self> {
        if !matches!(bytes.len(), 88 | 92) {
            return None;
        }
        let mut record = Self { bytes: [0; 92] };
        record.bytes[..bytes.len()].copy_from_slice(bytes);
        let version = record.word(4);
        if bytes.len() != if version == 5 { 92 } else { 88 } {
            return None;
        }
        let flags = record.word(24);
        let mask = (if version >= 4 { 1 << 12 } else { 0 })
            | 255
            | if version >= 2 { 1 << 8 } else { 0 }
            | if version >= 3 {
                (1 << 9) | (3 << 10)
            } else {
                0
            };
        if record.word(0) != 0x46504b45
            || !(1..=5).contains(&version)
            || !(1..=5).contains(&record.word(12))
            || record.word(16) > 1
            || !matches!(record.word(20), 3 | 5 | 9)
            || flags & 255 > 10
            || flags & !mask != 0
            || record.word(28) != crc32(&record.bytes[..28])
        {
            return None;
        }
        let crc_offset = if version >= 5 { 88 } else { 84 };
        if version >= 3
            && (record.word(32) > 10000
                || record.word(32) % 500 != 0
                || record.word(crc_offset) != crc32(&record.bytes[..crc_offset]))
        {
            return None;
        }
        Some(record)
    }
    fn preferences(&self) -> Preferences {
        let version = self.word(4);
        let flags = self.word(24);
        let mut out = Preferences::default();
        out.settings = Settings {
            brightness: self.word(12) as u8,
            imperial: self.word(16) != 0,
            levels: self.word(20) as u8,
            auto_off_minutes: (flags & 255) as u8,
            bluetooth: version == 1 || flags & (1 << 8) != 0,
            ..Settings::default()
        };
        if version >= 3 {
            out.settings.theme = if version >= 4 && flags & (1 << 12) != 0 {
                DashboardTheme::Classic
            } else {
                DashboardTheme::Standard
            };
            out.settings.language = ((flags >> 9) & 1) as u8;
            out.settings.power_mode = ((flags >> 10) & 3) as u8;
            out.settings.service_interval_km = self.word(32);
            out.since_service_m = self.word(36);
            out.odometer_m = self.word(40);
            if version >= 5 {
                out.trip_m = self.word(84);
            }
            for (index, error) in out.errors.iter_mut().enumerate() {
                *error = self.word(44 + index * 4);
            }
        }
        out
    }
    fn encode(preferences: &Preferences, sequence: u32) -> Self {
        let mut record = Self { bytes: [0; 92] };
        let settings = &preferences.settings;
        let flags = settings.auto_off_minutes as u32
            | ((settings.bluetooth as u32) << 8)
            | ((settings.language as u32) << 9)
            | ((settings.power_mode as u32) << 10)
            | ((settings.theme as u32) << 12);
        for (offset, value) in [
            (0, 0x46504b45),
            (4, 5),
            (8, sequence),
            (12, settings.brightness as u32),
            (16, settings.imperial as u32),
            (20, settings.levels as u32),
            (24, flags),
            (32, settings.service_interval_km),
            (36, preferences.since_service_m),
            (40, preferences.odometer_m),
            (84, preferences.trip_m),
        ] {
            record.put(offset, value);
        }
        record.put(28, crc32(&record.bytes[..28]));
        for (index, error) in preferences.errors.iter().enumerate() {
            record.put(44 + index * 4, *error);
        }
        record.put(88, crc32(&record.bytes[..88]));
        record
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveResult {
    Saved,
    Unchanged,
}
pub struct SettingsManager {
    store: SettingsStore,
    sequence: u32,
    last: Option<Preferences>,
    file_source: bool,
    last_current: bool,
    slot: usize,
}
impl SettingsManager {
    pub fn new(store: SettingsStore) -> Self {
        Self {
            store,
            sequence: 0,
            last: None,
            file_source: false,
            last_current: false,
            slot: 0,
        }
    }
    fn newer(a: u32, b: u32) -> bool {
        a.wrapping_sub(b) as i32 > 0
    }
    pub fn load(&mut self) -> Preferences {
        self.file_source = false;
        self.last_current = false;
        self.last = None;
        self.sequence = 0;
        self.slot = 0;
        let mut newest: Option<Record> = None;
        for (slot, path) in [
            (1, "/settings"),
            (0, "/settings.a"),
            (1, "/settings.b"),
            (0, "/prefs/a"),
            (1, "/prefs/b"),
        ] {
            if let Ok(bytes) = self.store.read(path) {
                if let Some(record) = Record::decode(&bytes) {
                    if newest
                        .as_ref()
                        .is_none_or(|old| Self::newer(record.word(8), old.word(8)))
                    {
                        self.last_current = record.word(4) == 5;
                        newest = Some(record);
                        self.slot = slot;
                        self.file_source = true;
                    }
                }
            }
        }
        if newest.is_none() {
            for address in [0x7fe000, 0x7ff000] {
                let mut bytes = [0; 88];
                if self.store.read_legacy(address, &mut bytes).is_ok() {
                    if let Some(record) = Record::decode(&bytes) {
                        if newest
                            .as_ref()
                            .is_none_or(|old| Self::newer(record.word(8), old.word(8)))
                        {
                            newest = Some(record);
                        }
                    }
                }
            }
        }
        if let Some(record) = newest {
            self.sequence = record.word(8);
            let out = record.preferences();
            self.last = Some(out);
            DebugUart::log(if self.file_source {
                "[SETTINGS] LOADED_LITTLEFS\n"
            } else {
                "[SETTINGS] LOADED_LEGACY\n"
            });
            out
        } else {
            Preferences::default()
        }
    }
    pub fn save(&mut self, preferences: &Preferences) -> Result<SaveResult, Error> {
        if self.file_source && self.last_current && self.last.as_ref() == Some(preferences) {
            return Ok(SaveResult::Unchanged);
        }
        let record = Record::encode(preferences, self.sequence.wrapping_add(1));
        if Record::decode(&record.bytes).is_none() {
            return Err(Error::InvalidConfig);
        }
        let target = 1 - self.slot;
        let path = if target == 0 { "/prefs/a" } else { "/prefs/b" };
        // Keep the other CRC-valid file untouched. This avoids relying on the
        // Rust filesystem's non-atomic rename fallback for durable settings.
        self.store.write_verified(path, &record.bytes)?;
        self.slot = target;
        self.sequence = record.word(8);
        self.last = Some(*preferences);
        self.file_source = true;
        self.last_current = true;
        DebugUart::log("[SETTINGS] SAVE_RESULT=00000000\n");
        Ok(SaveResult::Saved)
    }
}
