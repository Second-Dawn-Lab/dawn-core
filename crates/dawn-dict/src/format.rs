//! The dictionary file: entries as JSON records plus a sorted table of lowercase keys. A key is a headword or an
//! inflected form, and points at the entries it belongs to; looking a word up is a binary search, nothing is parsed
//! until an entry is asked for. The format is read-only after building.
//!
//! ```text
//! header (56 bytes): "DAWNDICT" | version u32 | source language [u8;4] | target language [u8;4]
//!                    | n_entries | n_keys | entry_offsets_at | entry_data_at | key_offsets_at | key_data_at
//!                    | posting_offsets_at | posting_data_at | file_length         (all u32, little endian)
//! entry_offsets: n_entries + 1 u32, into entry_data (one JSON record per entry)
//! key_offsets:   n_keys + 1 u32, into key_data (keys are sorted by their bytes)
//! posting_offsets: n_keys + 1 u32 (byte offsets), into posting_data: u32 words, the entry id with the top bit set
//!                  when the key reaches the entry through an inflected form rather than as its headword
//! ```

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use crate::entry::Entry;

const MAGIC: &[u8; 8] = b"DAWNDICT";
const VERSION: u32 = 1;
const HEADER: usize = 56;
/// Set in a posting when the key is an inflected form of the entry's headword.
pub const FORM_BIT: u32 = 0x8000_0000;

#[derive(Debug)]
pub enum DictError {
    Io(std::io::Error),
    /// Not a dictionary file, a newer version, or damaged.
    Format(&'static str),
    Json(serde_json::Error),
}

impl fmt::Display for DictError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Format(why) => write!(f, "not a usable dictionary file: {why}"),
            Self::Json(e) => write!(f, "damaged entry: {e}"),
        }
    }
}

impl std::error::Error for DictError {}

impl From<std::io::Error> for DictError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for DictError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

/// Marks that are written above or below a letter and are left out when comparing: Russian stress, Arabic and Urdu vowel
/// signs, combining accents. (Precomposed letters such as "ä" are letters and stay.)
fn is_mark(c: char) -> bool {
    matches!(c, '\u{0300}'..='\u{036F}' | '\u{0610}'..='\u{061A}' | '\u{064B}'..='\u{065F}' | '\u{0670}' | '\u{06D6}'..='\u{06ED}' | '\u{0640}')
}

/// How keys are compared: lowercase, without stress and vowel marks.
pub fn normalize(word: &str) -> String {
    word.chars().filter(|c| !is_mark(*c)).collect::<String>().to_lowercase()
}

/// Builds a dictionary file in memory.
pub struct Writer {
    source: String,
    target: String,
    entries: Vec<Vec<u8>>,
    keys: BTreeMap<String, Vec<u32>>,
}

impl Writer {
    pub fn new(source: &str, target: &str) -> Self {
        Self { source: source.into(), target: target.into(), entries: Vec::new(), keys: BTreeMap::new() }
    }

    /// Add an entry; its headword becomes a key. Returns its id.
    pub fn add_entry(&mut self, entry: &Entry) -> u32 {
        let id = self.entries.len() as u32;
        self.entries.push(serde_json::to_vec(entry).unwrap_or_default());
        self.add_key(&entry.head, id, false);
        id
    }

    /// Let `key` find entry `id` (as an inflected form when `form`).
    pub fn add_key(&mut self, key: &str, id: u32, form: bool) {
        let key = normalize(key);
        if key.is_empty() {
            return;
        }
        let word = if form { id | FORM_BIT } else { id };
        let postings = self.keys.entry(key).or_default();
        // A headword hit says more than a form hit for the same entry.
        if let Some(existing) = postings.iter_mut().find(|p| **p & !FORM_BIT == id) {
            if !form {
                *existing = id;
            }
            return;
        }
        postings.push(word);
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub fn key_count(&self) -> usize {
        self.keys.len()
    }

    pub fn finish(self) -> Vec<u8> {
        fn lang4(code: &str) -> [u8; 4] {
            let mut out = [0u8; 4];
            for (i, b) in code.bytes().take(4).enumerate() {
                out[i] = b;
            }
            out
        }
        let n_entries = self.entries.len();
        let n_keys = self.keys.len();
        let mut entry_offsets = Vec::with_capacity((n_entries + 1) * 4);
        let mut entry_data: Vec<u8> = Vec::new();
        for e in &self.entries {
            entry_offsets.extend((entry_data.len() as u32).to_le_bytes());
            entry_data.extend(e);
        }
        entry_offsets.extend((entry_data.len() as u32).to_le_bytes());

        let mut key_offsets = Vec::with_capacity((n_keys + 1) * 4);
        let mut key_data: Vec<u8> = Vec::new();
        let mut post_offsets = Vec::with_capacity((n_keys + 1) * 4);
        let mut post_data: Vec<u8> = Vec::new();
        for (key, postings) in &self.keys {
            key_offsets.extend((key_data.len() as u32).to_le_bytes());
            key_data.extend(key.as_bytes());
            post_offsets.extend((post_data.len() as u32).to_le_bytes());
            // Headword hits first.
            let mut sorted = postings.clone();
            sorted.sort_by_key(|p| (*p & FORM_BIT != 0, *p & !FORM_BIT));
            for p in sorted {
                post_data.extend(p.to_le_bytes());
            }
        }
        key_offsets.extend((key_data.len() as u32).to_le_bytes());
        post_offsets.extend((post_data.len() as u32).to_le_bytes());

        let entry_off_at = HEADER;
        let entry_data_at = entry_off_at + entry_offsets.len();
        let key_off_at = entry_data_at + entry_data.len();
        let key_data_at = key_off_at + key_offsets.len();
        let post_off_at = key_data_at + key_data.len();
        let post_data_at = post_off_at + post_offsets.len();
        let total = post_data_at + post_data.len();

        let mut out = Vec::with_capacity(total);
        out.extend(MAGIC);
        out.extend(VERSION.to_le_bytes());
        out.extend(lang4(&self.source));
        out.extend(lang4(&self.target));
        for v in [n_entries, n_keys, entry_off_at, entry_data_at, key_off_at, key_data_at, post_off_at, post_data_at, total] {
            out.extend((v as u32).to_le_bytes());
        }
        out.extend(entry_offsets);
        out.extend(entry_data);
        out.extend(key_offsets);
        out.extend(key_data);
        out.extend(post_offsets);
        out.extend(post_data);
        debug_assert_eq!(out.len(), total);
        out
    }
}

/// A dictionary file, read into memory.
pub struct Dict {
    data: Vec<u8>,
    source: String,
    target: String,
    n_entries: usize,
    n_keys: usize,
    entry_off_at: usize,
    entry_data_at: usize,
    key_off_at: usize,
    key_data_at: usize,
    post_off_at: usize,
    post_data_at: usize,
}

fn u32_at(data: &[u8], at: usize) -> Option<usize> {
    data.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
}

fn lang_from(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes.split(|b| *b == 0).next().unwrap_or(&[])).into_owned()
}

impl Dict {
    pub fn open(path: &Path) -> Result<Self, DictError> {
        Self::from_bytes(std::fs::read(path)?)
    }

    pub fn from_bytes(data: Vec<u8>) -> Result<Self, DictError> {
        if data.len() < HEADER || &data[..8] != MAGIC {
            return Err(DictError::Format("wrong header"));
        }
        if u32_at(&data, 8) != Some(VERSION as usize) {
            return Err(DictError::Format("unknown version"));
        }
        let field = |i: usize| u32_at(&data, 20 + i * 4).ok_or(DictError::Format("short header"));
        let (n_entries, n_keys) = (field(0)?, field(1)?);
        let (entry_off_at, entry_data_at, key_off_at, key_data_at, post_off_at, post_data_at, total) = (field(2)?, field(3)?, field(4)?, field(5)?, field(6)?, field(7)?, field(8)?);
        if total != data.len() || !(entry_off_at <= entry_data_at && entry_data_at <= key_off_at && key_off_at <= key_data_at && key_data_at <= post_off_at && post_off_at <= post_data_at && post_data_at <= total) {
            return Err(DictError::Format("sizes do not fit"));
        }
        if entry_data_at - entry_off_at != (n_entries + 1) * 4 || key_data_at - key_off_at != (n_keys + 1) * 4 || post_data_at - post_off_at != (n_keys + 1) * 4 {
            return Err(DictError::Format("tables do not fit"));
        }
        Ok(Self { source: lang_from(&data[12..16]), target: lang_from(&data[16..20]), n_entries, n_keys, entry_off_at, entry_data_at, key_off_at, key_data_at, post_off_at, post_data_at, data })
    }

    /// The language of the headwords.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The language of the translations.
    pub fn target(&self) -> &str {
        &self.target
    }

    pub fn entry_count(&self) -> usize {
        self.n_entries
    }

    pub fn key_count(&self) -> usize {
        self.n_keys
    }

    fn key(&self, i: usize) -> &[u8] {
        let a = u32_at(&self.data, self.key_off_at + i * 4).unwrap_or(0);
        let b = u32_at(&self.data, self.key_off_at + (i + 1) * 4).unwrap_or(a);
        self.data.get(self.key_data_at + a..self.key_data_at + b).unwrap_or(&[])
    }

    /// The key at position `i` of the sorted table.
    pub fn key_str(&self, i: usize) -> String {
        String::from_utf8_lossy(self.key(i)).into_owned()
    }

    /// The position of the first key that is not smaller than `key`.
    pub fn lower_bound(&self, key: &[u8]) -> usize {
        let (mut lo, mut hi) = (0, self.n_keys);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.key(mid) < key {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }

    /// The entries a key leads to as `(entry id, reached through an inflected form)`. The key is normalized.
    pub fn postings(&self, word: &str) -> Vec<(u32, bool)> {
        let key = normalize(word);
        let i = self.lower_bound(key.as_bytes());
        if i >= self.n_keys || self.key(i) != key.as_bytes() {
            return Vec::new();
        }
        self.postings_at(i)
    }

    pub fn postings_at(&self, i: usize) -> Vec<(u32, bool)> {
        let a = u32_at(&self.data, self.post_off_at + i * 4).unwrap_or(0);
        let b = u32_at(&self.data, self.post_off_at + (i + 1) * 4).unwrap_or(a);
        let mut out = Vec::new();
        let mut at = self.post_data_at + a;
        while at + 4 <= self.post_data_at + b {
            let word = u32_at(&self.data, at).unwrap_or(0) as u32;
            out.push((word & !FORM_BIT, word & FORM_BIT != 0));
            at += 4;
        }
        out
    }

    pub fn entry(&self, id: u32) -> Result<Entry, DictError> {
        let id = id as usize;
        if id >= self.n_entries {
            return Err(DictError::Format("no such entry"));
        }
        let a = u32_at(&self.data, self.entry_off_at + id * 4).ok_or(DictError::Format("short table"))?;
        let b = u32_at(&self.data, self.entry_off_at + (id + 1) * 4).ok_or(DictError::Format("short table"))?;
        let bytes = self.data.get(self.entry_data_at + a..self.entry_data_at + b).ok_or(DictError::Format("entry out of range"))?;
        Ok(serde_json::from_slice(bytes)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::{Sense, Word};

    fn entry(head: &str, translation: &str) -> Entry {
        Entry { head: head.into(), pos: "noun".into(), senses: vec![Sense { gloss: translation.into(), words: vec![Word { word: translation.into(), note: String::new() }], tags: vec![] }], ..Default::default() }
    }

    fn sample() -> Dict {
        let mut w = Writer::new("de", "en");
        let haus = w.add_entry(&entry("Haus", "house"));
        w.add_key("Häuser", haus, true);
        w.add_key("Hauses", haus, true);
        let hund = w.add_entry(&entry("Hund", "dog"));
        w.add_key("Hunde", hund, true);
        w.add_key("haus", haus, true); // the headword wins over a form hit
        Dict::from_bytes(w.finish()).unwrap()
    }

    #[test]
    fn keys_find_their_entries_by_headword_or_form_ignoring_case() {
        let d = sample();
        assert_eq!((d.source(), d.target(), d.entry_count(), d.key_count()), ("de", "en", 2, 5));
        assert_eq!(d.postings("HAUS"), vec![(0, false)]);
        assert_eq!(d.postings("häuser"), vec![(0, true)]);
        assert_eq!(d.postings("Hunde"), vec![(1, true)]);
        assert!(d.postings("Katze").is_empty());
        assert_eq!(d.entry(1).unwrap().head, "Hund");
        assert!(d.entry(2).is_err());
    }

    #[test]
    fn stress_and_vowel_marks_do_not_matter_when_looking_up() {
        assert_eq!(normalize("Да\u{301}ча"), "дача");
        assert_eq!(normalize("\u{0643}\u{0650}\u{062A}\u{064E}\u{0627}\u{0628}"), "\u{0643}\u{062A}\u{0627}\u{0628}");
        assert_eq!(normalize("Häuser"), "häuser");
        let mut w = Writer::new("ru", "en");
        let id = w.add_entry(&entry("да\u{301}ча", "dacha"));
        let d = Dict::from_bytes(w.finish()).unwrap();
        assert_eq!(d.postings("дача"), vec![(id, false)]);
        assert_eq!(d.postings("ДА\u{301}ЧА"), vec![(id, false)]);
    }

    #[test]
    fn the_sorted_table_supports_prefix_ranges() {
        let d = sample();
        let start = d.lower_bound(b"hau");
        assert_eq!(d.key_str(start), "haus");
        assert_eq!(d.key_str(start + 1), "hauses");
        assert_eq!(d.key_str(d.lower_bound(b"zzz").saturating_sub(1)), "häuser");
    }

    #[test]
    fn damaged_or_foreign_files_are_refused_not_trusted() {
        assert!(Dict::from_bytes(b"not a dictionary at all, not a dictionary at all, nope....".to_vec()).is_err());
        let mut bytes = Writer::new("de", "en").finish();
        assert!(Dict::from_bytes(bytes.clone()).is_ok());
        bytes.truncate(bytes.len() - 1);
        assert!(Dict::from_bytes(bytes).is_err(), "length no longer matches the header");
        let mut changed = Writer::new("de", "en").finish();
        changed[8] = 9;
        assert!(Dict::from_bytes(changed).is_err());
    }
}
