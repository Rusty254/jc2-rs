//! The text database: every localized string of the game (UI, mission titles and
//! briefings, dialogue subtitles, settlement names, credits) in one CSV,
//! `text\ms15text.csv` — about 11 200 rows and 6.3 MB.
//!
//! Layout (see `research/formats/text.md`): one row per line, every field wrapped in
//! `#D#` marks and the fields joined by a comma, so a row reads
//!
//! ```text
//! #D#set_mil_txt_11#D#,#D#Lembah Genting Tinggi#D#,#D#…fr#D#,#D#…it#D#,… 8 languages
//! ```
//!
//! The first field is the key (case as the authors typed it, looked up without regard
//! to case here), then the English, French, Italian, German, Spanish, Russian, Polish
//! and Japanese texts. Bytes are UTF-8 (one field in the file holds a stray Windows-1252
//! ellipsis). Rows end with a run of commas (a spreadsheet export). Dialogue lines
//! begin with subtitle cues, `{0 - 2.5}`, see [`strip_cues`]. The DLC packs ship the same
//! thing in `<n>_locator.csv` (`No localization` where a language is missing).
//!
//! `settings\locator.bin` names the file (`Text/ms15text.csv`) and
//! `Text/Escape_sequences.csv` for the button glyph escapes (`{EXECUTE}`).

use crate::GameFs;
use std::collections::HashMap;

/// The file's languages, in column order. Codes are the usual two letters.
pub const LANGUAGES: [(&str, &str); 8] = [
    ("en", "English"),
    ("fr", "French"),
    ("it", "Italian"),
    ("de", "German"),
    ("es", "Spanish"),
    ("ru", "Russian"),
    ("pl", "Polish"),
    ("ja", "Japanese"),
];

/// The text file in the game's file system.
pub const TEXT_FILE: &str = "text\\ms15text.csv";

/// What the DLC rows say where they have no translation.
const NO_LOCALIZATION: &str = "No localization";

/// The column of a language given as a code (`en`) or a name (`English`), any case.
pub fn language_index(lang: &str) -> Option<usize> {
    let l = lang.trim().to_ascii_lowercase();
    // `eng`, `jp`, `ger`, `fre`, … as the original's file names use them
    let l = match l.as_str() {
        "eng" => "en",
        "fre" | "fra" => "fr",
        "ita" => "it",
        "ger" | "deu" => "de",
        "spa" => "es",
        "rus" => "ru",
        "pol" => "pl",
        "jp" | "jpn" => "ja",
        other => other,
    };
    LANGUAGES
        .iter()
        .position(|(c, n)| *c == l || n.to_ascii_lowercase() == l)
}

/// Key → string for one language.
#[derive(Clone, Debug, Default)]
pub struct TextDb {
    /// Column in [`LANGUAGES`].
    pub language: usize,
    /// Lower-cased key → text.
    map: HashMap<String, String>,
}

impl TextDb {
    /// Parse a text CSV, keeping the text of language column `language` (0 = English)
    /// and falling back to English where that column is empty or says "No localization".
    /// A key that occurs twice keeps its first row.
    pub fn parse(data: &[u8], language: usize) -> Self {
        let mut db = Self {
            language,
            map: HashMap::new(),
        };
        db.add(data);
        db
    }

    /// Add the rows of another file (a DLC's `_locator.csv`); keys already present win.
    pub fn add(&mut self, data: &[u8]) {
        for line in data.split(|&b| b == b'\n') {
            let Some(line) = line.strip_prefix(b"#D#") else {
                continue;
            };
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            let mut fields = line.split_slice(b"#D#,#D#");
            let Some(key) = fields.next() else { continue };
            let key = decode(key).trim().to_ascii_lowercase();
            if key.is_empty() || self.map.contains_key(&key) {
                continue;
            }
            let texts: Vec<&[u8]> = fields.collect();
            let field = |i: usize| -> Option<String> {
                let mut f = *texts.get(i)?;
                // the last field carries the closing mark and the spreadsheet's commas
                if let Some(p) = find(f, b"#D#") {
                    f = &f[..p];
                }
                let s = decode(f);
                let s = s.trim();
                (!s.is_empty() && s != NO_LOCALIZATION).then(|| s.to_string())
            };
            if let Some(s) = field(self.language).or_else(|| field(0)) {
                self.map.insert(key, s);
            }
        }
    }

    /// Read the game's text: `text\ms15text.csv` and the DLC packs' `<n>_locator.csv`.
    /// `language` is a code or name ([`language_index`]); an unknown one means English.
    /// `None` when the main file is not there.
    pub fn load(fs: &GameFs, language: &str) -> Option<Self> {
        let lang = language_index(language).unwrap_or(0);
        let mut db = Self::parse(&fs.read_inflated(TEXT_FILE).ok()?, lang);
        for n in 1..=40 {
            if let Ok(d) = fs.read_inflated(&format!("{n}_locator.csv")) {
                db.add(&d);
            }
        }
        Some(db)
    }

    /// The text of a key (any case).
    pub fn get(&self, key: &str) -> Option<&str> {
        if key.bytes().any(|b| b.is_ascii_uppercase()) {
            self.map.get(&key.to_ascii_lowercase())
        } else {
            self.map.get(key)
        }
        .map(String::as_str)
    }

    /// The text of a key, or the key itself where there is none.
    pub fn get_or_key<'a>(&'a self, key: &'a str) -> &'a str {
        self.get(key).unwrap_or(key)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// All rows, in no particular order (keys lower case).
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.map.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// The English words the HUD uses for the player's weapons, by the keys that hold
    /// them (`gui_txt_59`…`gui_txt_68`: pistol, SMG, assault rifle, shotgun, sniper
    /// rifle, rocket launcher, machine gun). A convenience for HUD code.
    pub fn weapon_name(&self, weapon: &str) -> Option<&str> {
        let key = match weapon.to_ascii_lowercase().as_str() {
            "pistol" | "beretta" => "gui_txt_59",
            "smg" | "submachinegun" | "submachine gun" => "gui_txt_61",
            "assault" | "assault rifle" => "gui_txt_64",
            "shotgun" => "gui_txt_65",
            "sniper" | "sniper rifle" => "gui_txt_66",
            "rocket" | "rocket launcher" => "gui_txt_67",
            "machine gun" | "machinegun" => "gui_txt_68",
            _ => return None,
        };
        self.get(key)
    }
}

/// The text with its subtitle cues (`{0 - 2.5}`) removed, whitespace tidied.
pub fn strip_cues(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('{') {
        let (before, after) = rest.split_at(i);
        let Some(j) = after.find('}') else { break };
        let cue = &after[1..j];
        if cue.chars().all(|c| c.is_ascii_digit() || " .-".contains(c)) {
            out.push_str(before);
            rest = &after[j + 1..];
        } else {
            out.push_str(&rest[..i + j + 1]);
            rest = &after[j + 1..];
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Splitting a byte slice at a multi-byte separator.
trait SplitSlice {
    fn split_slice<'a>(&'a self, sep: &'a [u8]) -> SliceSplit<'a>;
}

impl SplitSlice for [u8] {
    fn split_slice<'a>(&'a self, sep: &'a [u8]) -> SliceSplit<'a> {
        SliceSplit {
            rest: Some(self),
            sep,
        }
    }
}

struct SliceSplit<'a> {
    rest: Option<&'a [u8]>,
    sep: &'a [u8],
}

impl<'a> Iterator for SliceSplit<'a> {
    type Item = &'a [u8];
    fn next(&mut self) -> Option<&'a [u8]> {
        let r = self.rest?;
        match find(r, self.sep) {
            Some(i) => {
                self.rest = Some(&r[i + self.sep.len()..]);
                Some(&r[..i])
            }
            None => {
                self.rest = None;
                Some(r)
            }
        }
    }
}

/// UTF-8, or Windows-1252 for the odd field that is not.
fn decode(b: &[u8]) -> String {
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => b.iter().map(|&c| cp1252(c)).collect(),
    }
}

fn cp1252(c: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
        '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}',
        'ž', 'Ÿ',
    ];
    match c {
        0x80..=0x9f => HIGH[(c - 0x80) as usize],
        _ => c as char,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = b"#D#Set_Mil_Txt_1#D#,#D#Base One #D#,#D#Base Un#D#,#D##D#,#D#Basis#D#,#D#B#D#,#D#B#D#,#D#B#D#,#D#B#D#,,,,,,\r\n\
#D#dlc_x#D#,#D#Only English#D#,#D#No localization#D#,#D#No localization#D#,#D#No localization#D#,#D#No localization#D#,#D#No localization#D#,#D#No localization#D#,#D#No localization#D#\n\
#D#dup#D#,#D#first#D#,#D#a#D#\n#D#dup#D#,#D#second#D#,#D#b#D#\n";

    #[test]
    fn rows_split_and_fall_back_to_english() {
        let en = TextDb::parse(SAMPLE, 0);
        assert_eq!(en.get("set_mil_txt_1"), Some("Base One"));
        assert_eq!(en.get("SET_MIL_TXT_1"), Some("Base One"));
        assert_eq!(en.get("dup"), Some("first"));
        let fr = TextDb::parse(SAMPLE, 1);
        assert_eq!(fr.get("set_mil_txt_1"), Some("Base Un"));
        assert_eq!(fr.get("dlc_x"), Some("Only English"));
        // an empty column falls back too
        assert_eq!(
            TextDb::parse(SAMPLE, 2).get("set_mil_txt_1"),
            Some("Base One")
        );
        assert_eq!(TextDb::parse(SAMPLE, 3).get("set_mil_txt_1"), Some("Basis"));
        assert_eq!(en.len(), 3);
    }

    #[test]
    fn languages_and_cues() {
        assert_eq!(language_index("EN"), Some(0));
        assert_eq!(language_index("german"), Some(3));
        assert_eq!(language_index("jp"), Some(7));
        assert_eq!(language_index("klingon"), None);
        assert_eq!(
            strip_cues("{0 - 2.5}Ah, you come to race? {2.7 - 8.1}Then {EXECUTE}!"),
            "Ah, you come to race? Then {EXECUTE}!"
        );
        assert_eq!(decode(b"a\x85b"), "a…b");
    }

    /// Needs JC2_TEST_DIR: the settlement names the map shows.
    #[test]
    fn the_settlements_have_names() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = GameFs::open(dir).unwrap();
        let Some(db) = TextDb::load(&fs, "en") else {
            return;
        };
        assert!(db.len() > 10_000, "{}", db.len());
        assert_eq!(db.get("set_mil_txt_11"), Some("Lembah Genting Tinggi"));
        assert_eq!(db.get("set_civ_txt_134"), Some("Pekan Gua Cina"));
        assert_eq!(db.get("f1s07_title_txt"), Some("Pelabuhan Saudagar Harbor"));
        assert_eq!(db.weapon_name("assault"), Some("Assault Rifle"));
        let de = TextDb::load(&fs, "de").unwrap();
        assert_eq!(de.get("f1s07_title_txt"), Some("Pelabuhan Saudagar-Hafen"));
        assert!(de.len() > 11_000);
        // every settlement's name key resolves
        let (Ok(s), Ok(i), Ok(t)) = (
            fs.read_inflated("settlements.bin"),
            fs.read_inflated("icons_global.bin"),
            fs.read_inflated("icon_types.bin"),
        ) else {
            return;
        };
        let joined = crate::icons::settlement_icons(
            &crate::icons::icon_types(&t).unwrap(),
            &crate::icons::map_icons(&i).unwrap(),
            &crate::icons::settlements(&s).unwrap(),
        );
        let missing: Vec<_> = joined
            .iter()
            .filter(|s| db.get(&s.text).is_none())
            .map(|s| format!("{} {}", s.id, s.text))
            .collect();
        assert!(missing.is_empty(), "no name for {missing:?}");
    }
}
