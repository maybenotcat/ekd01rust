//! Optional per-family font sizes and character coverage for embedded raster fonts.
use std::collections::HashMap;

struct Profile {
    sizes: Vec<i16>,
    characters: Option<String>,
}

pub struct AtlasPolicy {
    profiles: HashMap<String, Profile>,
}

impl AtlasPolicy {
    pub fn load() -> Result<Self, String> {
        let mut profiles = HashMap::new();
        if let Ok(path) = std::env::var("SLINT_FONT_ATLAS_CONFIG") {
            println!("cargo:rerun-if-env-changed=SLINT_FONT_ATLAS_CONFIG");
            println!("cargo:rerun-if-changed={path}");
            let contents =
                std::fs::read_to_string(&path).map_err(|error| format!("{path}: {error}"))?;
            for (line_number, line) in contents.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let fields: Vec<_> = line.splitn(3, '|').collect();
                if fields.len() != 3 || fields[0].is_empty() {
                    return Err(format!(
                        "Invalid font atlas profile at {path}:{}",
                        line_number + 1
                    ));
                }
                let mut sizes = fields[1]
                    .split(',')
                    .map(|size| size.parse::<i16>())
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| error.to_string())?;
                if sizes.is_empty() || sizes.iter().any(|size| *size <= 0) {
                    return Err(format!("Invalid font atlas sizes for {}", fields[0]));
                }
                sizes.sort_unstable();
                sizes.dedup();
                let characters = (fields[2] != "*").then(|| fields[2].to_owned());
                if profiles
                    .insert(fields[0].to_owned(), Profile { sizes, characters })
                    .is_some()
                {
                    return Err(format!("Duplicate font atlas family: {}", fields[0]));
                }
            }
        }
        Ok(Self { profiles })
    }

    pub fn select(
        &self,
        family: &str,
        scale: f64,
        default_sizes: &[i16],
        default_characters: impl Iterator<Item = char>,
    ) -> (Vec<i16>, Vec<char>) {
        let (sizes, mut characters): (Vec<i16>, Vec<char>) =
            if let Some(profile) = self.profiles.get(family) {
                let sizes = profile
                    .sizes
                    .iter()
                    .map(|size| (*size as f64 * scale).round() as i16)
                    .collect();
                let characters = match &profile.characters {
                    Some(characters) => characters.chars().collect(),
                    None => default_characters.collect(),
                };
                (sizes, characters)
            } else {
                (default_sizes.to_vec(), default_characters.collect())
            };
        characters.sort_unstable();
        characters.dedup();
        (sizes, characters)
    }

    pub fn report(font: &crate::embedded_resources::BitmapFont) -> String {
        let characters: String = font
            .character_map
            .iter()
            .map(|entry| entry.code_point)
            .collect();
        font.glyphs
            .iter()
            .map(|atlas| {
                format!(
                    "{}\t{}\t{}\t{}\t{}\t{}\n",
                    font.family_name,
                    atlas.pixel_size,
                    font.character_map.len(),
                    atlas
                        .glyph_data
                        .iter()
                        .map(|glyph| glyph.data.len())
                        .sum::<usize>(),
                    font.sdf,
                    characters
                )
            })
            .collect()
    }

    pub fn write_report(report: &str) -> Result<(), String> {
        if let Ok(path) = std::env::var("SLINT_FONT_ATLAS_REPORT") {
            let path = std::path::Path::new(&path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            std::fs::write(path, report).map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}
