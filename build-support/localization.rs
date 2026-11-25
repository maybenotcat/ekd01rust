//! Build the locale registry from data; no translation code runs on the device.
use std::{collections::BTreeSet, fs, path::Path};

#[derive(Debug, PartialEq, Eq)]
pub struct Locale {
    pub id: u8,
    pub code: String,
    pub name: String,
}

pub fn parse_registry(text: &str) -> Result<Vec<Locale>, String> {
    let mut locales = Vec::new();
    let mut ids = BTreeSet::new();
    let mut codes = BTreeSet::new();
    for line in text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let fields: Vec<_> = line.split('|').map(str::trim).collect();
        if fields.len() != 3 {
            return Err(format!("Expected ID | code | name: {line}"));
        }
        let id = fields[0]
            .parse::<u8>()
            .map_err(|_| format!("Invalid locale ID: {line}"))?;
        let code = fields[1];
        if code.is_empty()
            || !code
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            || fields[2].is_empty()
            || !ids.insert(id)
            || !codes.insert(code.to_owned())
        {
            return Err(format!("Invalid or duplicate locale: {line}"));
        }
        locales.push(Locale {
            id,
            code: code.into(),
            name: fields[2].into(),
        });
    }
    if !locales.first().is_some_and(|l| l.id == 0 && l.code == "en") {
        return Err("The fallback locale must be English with permanent ID 0".into());
    }
    if !locales.iter().any(|l| l.id == 1 && l.code == "ru") {
        return Err("Retain Russian with permanent ID 1 for existing settings".into());
    }
    Ok(locales)
}

pub fn generate(root: &Path) {
    let directory = root.join("translations");
    println!("cargo:rerun-if-changed={}", directory.display());
    let locales = parse_registry(
        &fs::read_to_string(directory.join("languages.txt")).expect("locale registry"),
    )
    .expect("valid stable locale registry");
    for locale in &locales {
        if locale.code != "en" {
            assert!(
                directory
                    .join(&locale.code)
                    .join("LC_MESSAGES/ekd01rust.po")
                    .is_file(),
                "Missing gettext catalog for {}",
                locale.code
            );
        }
    }
    // Refuse catalogs that would be bundled by Slint but absent from the selector.
    for entry in fs::read_dir(&directory).expect("translation directory") {
        let path = entry.expect("translation entry").path();
        if path.join("LC_MESSAGES/ekd01rust.po").exists() {
            assert!(
                locales
                    .iter()
                    .any(|l| path.file_name().unwrap() == l.code.as_str()),
                "Catalog is missing from languages.txt: {}",
                path.display()
            );
        }
    }
    let mut source = String::from(
        "// Generated from translations/languages.txt.\npub const LANGUAGES: &[Language] = &[\n",
    );
    for locale in locales {
        source.push_str(&format!(
            "    Language {{ id: {}, code: {:?}, name: {:?} }},\n",
            locale.id, locale.code, locale.name
        ));
    }
    source.push_str("];\n");
    let path = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("languages.rs");
    if fs::read_to_string(&path).ok().as_deref() != Some(&source) {
        fs::write(path, source).expect("generated locale registry");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_accepts_more_languages_and_preserves_ids() {
        let locales = parse_registry("0|en|English\n1|ru|Русский\n42|de|Deutsch").unwrap();
        assert_eq!(locales.len(), 3);
        assert_eq!(locales[2].id, 42);
        assert!(parse_registry("0|en|English\n1|ru|Русский\n1|de|Deutsch").is_err());
        assert!(parse_registry("0|en|English\n1|ru|Русский\n2|ru|Russian").is_err());
        assert!(parse_registry("0|en|English\n2|ru|Русский").is_err());
        assert!(parse_registry("1|ru|Русский\n0|en|English").is_err());
    }
}
