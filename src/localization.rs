//! Stable locale identities, independent of translated strings and catalog order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Language {
    pub id: u8,
    pub code: &'static str,
    pub name: &'static str,
}
include!(concat!(env!("OUT_DIR"), "/languages.rs"));

impl Language {
    pub fn from_id(id: u8) -> &'static Self {
        LANGUAGES
            .iter()
            .find(|l| l.id == id)
            .unwrap_or(&LANGUAGES[0])
    }
    pub fn next(id: u8, direction: i32) -> u8 {
        let position = LANGUAGES.iter().position(|l| l.id == id).unwrap_or(0);
        let next = (position as i32 + direction.signum()).rem_euclid(LANGUAGES.len() as i32);
        LANGUAGES[next as usize].id
    }
}
