//! Reads kaikki.org extracts (Wiktionary parsed by wiktextract, one JSON object per line) into dictionary entries.
//!
//! Two kinds of file are used, both from the **English edition** of Wiktionary:
//!
//! * the words of a language with English glosses (`kaikki.org-dictionary-German.jsonl`): the "German to English"
//!   direction. Entries that only say "inflection of X" are not entries; they become keys that lead to X, next to the
//!   `forms` tables of the base forms, which is how "Häuser" finds "Haus" and "ging" finds "gehen".
//! * the English words with their `translations` into a language (`kaikki.org-dictionary-English.jsonl`): the
//!   "English to German" direction.
//!
//! The data is Wiktionary's and carries Wiktionary's license (CC BY-SA, with GFDL); see `docs/dictionary.md`.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::entry::{Entry, Example, Sense, Word};
use crate::format::Writer;

/// Parts of speech that are not words a reader looks up.
const SKIPPED_POS: &[&str] = &["character", "punct", "symbol"];

/// A part of speech that is no word to look up. In Chinese and Japanese a single character is a word ("character" in Wiktionary).
fn skipped(lang: &str, pos: &str) -> bool {
    SKIPPED_POS.contains(&pos) && !(pos == "character" && matches!(lang, "zh" | "ja"))
}

/// Tags that mark a translation as regional, historical or otherwise not the plain standard word.
fn is_noise_tag(tag: &str) -> bool {
    let t = tag.to_lowercase();
    t.contains("german") && t != "german" // Alemannic-German, Swiss German, Austrian German, ...
        || t.contains("dialect")
        || ["obsolete", "archaic", "dated", "rare", "vulgar", "slang", "derogatory", "offensive", "colloquial", "regional", "poetic", "humorous", "Austria", "Switzerland", "Bavaria", "Berlin", "Low German"].iter().any(|n| t == n.to_lowercase())
}

/// Tags worth showing next to a sense.
fn is_label(tag: &str) -> bool {
    matches!(tag, "informal" | "formal" | "figurative" | "literary" | "colloquial" | "dated" | "obsolete" | "archaic" | "rare" | "slang" | "vulgar" | "derogatory" | "offensive" | "humorous" | "regional" | "transitive" | "intransitive" | "reflexive" | "usually" | "plural" | "uncountable")
}

fn gender_note(tags: &[String]) -> String {
    let has = |t: &str| tags.iter().any(|x| x == t);
    let mut parts: Vec<&str> = Vec::new();
    if has("masculine") {
        parts.push("m");
    }
    if has("feminine") {
        parts.push("f");
    }
    if has("neuter") {
        parts.push("n");
    }
    if has("plural") {
        parts.push("pl");
    }
    parts.join("/")
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// The inflected forms of an entry worth a key: single words, not table metadata.
fn forms_of(v: &Value, head: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for f in v.get("forms").and_then(Value::as_array).into_iter().flatten() {
        let form = str_of(f, "form").trim();
        let tags = strings(f.get("tags"));
        let meta = tags.iter().any(|t| t == "table-tags" || t == "inflection-template" || t == "class" || t == "romanization");
        if meta || form.is_empty() || form == head || form.len() > 40 || form.contains([' ', '{', '|']) || (form.starts_with('-') && !head.starts_with('-')) {
            continue;
        }
        if !out.iter().any(|o| o == form) {
            out.push(form.to_string());
        }
    }
    out
}

/// The head line without the long tail: "Haus n (strong, genitive Hauses, plural Häuser, diminutive Häuschen n or ...)" keeps
/// what a learner needs (gender, genitive, plural) and drops what follows once the line is longer than `MAX_GRAMMAR`.
const MAX_GRAMMAR: usize = 120;

pub fn shorten_grammar(line: &str) -> String {
    if line.chars().count() <= MAX_GRAMMAR {
        return line.to_string();
    }
    let cut: String = line.chars().take(MAX_GRAMMAR).collect();
    // Back to the last comma outside any bracket but the one that holds the forms, then close that bracket.
    let mut depth = 0i32;
    let mut last_comma = None;
    for (i, c) in cut.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 1 => last_comma = Some(i),
            _ => {}
        }
    }
    match last_comma {
        Some(i) => format!("{})", &cut[..i]),
        None => format!("{}\u{2026}", cut.trim_end()),
    }
}

/// A gloss in one line: the long explanations some entries have are cut at a word boundary.
const MAX_GLOSS: usize = 150;

pub fn shorten_gloss(gloss: &str) -> String {
    let gloss = gloss.trim();
    if gloss.chars().count() <= MAX_GLOSS {
        return gloss.to_string();
    }
    let cut: String = gloss.chars().take(MAX_GLOSS).collect();
    let at = cut.rfind(' ').unwrap_or(cut.len());
    format!("{}\u{2026}", cut[..at].trim_end_matches([',', ';', ' ']))
}

fn grammar_of(v: &Value, head: &str) -> String {
    let line = v.get("head_templates").and_then(Value::as_array).and_then(|a| a.first()).map(|t| str_of(t, "expansion")).unwrap_or("");
    // A bare repetition of the headword says nothing.
    if line == head || line.contains("{{") {
        String::new()
    } else {
        shorten_grammar(line)
    }
}

/// One gloss from a sense: the last of the list (the parent glosses are context lines such as "inflection of x:").
fn gloss_of(sense: &Value) -> String {
    let glosses = strings(sense.get("glosses"));
    shorten_gloss(glosses.last().map(String::as_str).unwrap_or(""))
}

/// The gender in a head line: `Haus n (strong, ...)` gives "n", `Student m (...)` "m". Empty when there is none.
fn gender_from_grammar(head: &str, grammar: &str) -> String {
    let rest = grammar.strip_prefix(head).unwrap_or(grammar).trim_start();
    let first = rest.split([' ', ',']).next().unwrap_or("");
    match first {
        "m" | "f" | "n" | "p" => {
            // "m or f", "m or n"
            let mut parts = vec![first.replace('p', "pl")];
            let after: Vec<&str> = rest.split_whitespace().skip(1).take(2).collect();
            if after.len() == 2 && after[0] == "or" && matches!(after[1], "m" | "f" | "n") {
                parts.push(after[1].to_string());
            }
            parts.join("/")
        }
        _ => String::new(),
    }
}

/// An English word that a German gloss gives for a meaning ("house" in "house, building"), or none when the piece is
/// an explanation rather than a word.
fn english_item(piece: &str, pos: &str) -> Option<String> {
    let piece = piece.trim().trim_end_matches('.');
    if piece.is_empty() || piece.chars().count() > 28 || piece.contains(['(', ')', '[', ']', ':', '"', '/', '\u{2018}', '\u{2019}']) {
        return None;
    }
    let mut word = piece;
    if pos == "verb" {
        word = word.strip_prefix("to ").unwrap_or(word);
    } else if pos == "noun" {
        for article in ["a ", "an ", "the "] {
            if let Some(rest) = word.strip_prefix(article) {
                word = rest;
            }
        }
    }
    let word = word.trim();
    if word.is_empty() || word.split_whitespace().count() > 3 || !word.chars().next().is_some_and(char::is_alphabetic) {
        return None;
    }
    Some(word.to_string())
}

fn examples_of(senses: &[Value], max: usize) -> Vec<Example> {
    let mut out = Vec::new();
    for sense in senses {
        for ex in sense.get("examples").and_then(Value::as_array).into_iter().flatten() {
            let text = str_of(ex, "text").trim();
            let english = if !str_of(ex, "english").is_empty() { str_of(ex, "english") } else { str_of(ex, "translation") };
            // Only real examples with their translation, short enough for a popup.
            if text.is_empty() || english.is_empty() || str_of(ex, "type") == "quotation" || text.chars().count() > 140 || english.chars().count() > 160 {
                continue;
            }
            out.push(Example { text: text.to_string(), translation: english.trim().to_string() });
            if out.len() >= max {
                return out;
            }
        }
    }
    out
}

/// What the reader collects before the dictionary file is written.
#[derive(Default)]
pub struct Collector {
    entries: Vec<(Entry, Vec<String>)>,
    /// `(inflected word, base word)` from entries that only say "inflection of ...".
    links: Vec<(String, String)>,
    /// English words derived from the German entries' glosses ("house" from "Haus: house, building"), the reverse
    /// direction for words the English file has no translations for, in order of appearance.
    reverse: Vec<Entry>,
    reverse_index: HashMap<(String, String), usize>,
    /// Lowercase headwords of `reverse`, so the English file is only read closely for those.
    reverse_heads: HashSet<String>,
    /// Inflected forms of English words from the English file, by lowercase headword.
    english_forms: HashMap<String, Vec<String>>,
    english_links: Vec<(String, String)>,
    pub lines: u64,
    pub skipped_lines: u64,
}

impl Collector {
    pub fn new() -> Self {
        Self::default()
    }

    /// A line of the "words of a language with English glosses" file. `lang_code` is the language of the headwords.
    pub fn feed_glosses(&mut self, line: &str, lang_code: &str) {
        self.lines += 1;
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            self.skipped_lines += 1;
            return;
        };
        if str_of(&v, "lang_code") != lang_code {
            return;
        }
        let head = str_of(&v, "word").trim().to_string();
        let pos = str_of(&v, "pos").to_string();
        if head.is_empty() || skipped(lang_code, &pos) {
            return;
        }
        let senses: Vec<Value> = v.get("senses").and_then(Value::as_array).cloned().unwrap_or_default();
        let (inflections, own): (Vec<&Value>, Vec<&Value>) = senses.iter().partition(|s| s.get("form_of").is_some());
        for s in &inflections {
            for target in s.get("form_of").and_then(Value::as_array).into_iter().flatten() {
                let base = str_of(target, "word").trim();
                if !base.is_empty() && base != head {
                    self.links.push((head.clone(), base.to_string()));
                }
            }
        }
        let mut out_senses: Vec<Sense> = Vec::new();
        for s in own {
            let gloss = gloss_of(s);
            if gloss.is_empty() || gloss == "no-gloss" {
                continue;
            }
            let tags: Vec<String> = strings(s.get("tags")).into_iter().filter(|t| is_label(t)).take(3).collect();
            out_senses.push(Sense { gloss, words: Vec::new(), tags });
            if out_senses.len() >= 12 {
                break;
            }
        }
        if out_senses.is_empty() {
            return;
        }
        let entry = Entry { grammar: grammar_of(&v, &head), examples: examples_of(&senses, 2), head: head.clone(), pos, senses: out_senses };
        let forms = forms_of(&v, &head);
        self.entries.push((entry, forms));
    }

    /// A line of the German (or other) file for the reverse direction: each English word its glosses give leads to this entry.
    pub fn feed_reverse(&mut self, line: &str, lang: &str) {
        self.lines += 1;
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            self.skipped_lines += 1;
            return;
        };
        if str_of(&v, "lang_code") != lang {
            return;
        }
        let head = str_of(&v, "word").trim().to_string();
        let pos = str_of(&v, "pos").to_string();
        if head.is_empty() || skipped(lang, &pos) || head.contains(' ') && pos == "phrase" {
            return;
        }
        let note = gender_from_grammar(&head, &grammar_of(&v, &head));
        for sense in v.get("senses").and_then(Value::as_array).into_iter().flatten() {
            if sense.get("form_of").is_some() {
                continue;
            }
            let full = strings(sense.get("glosses")).pop().unwrap_or_default();
            if full.is_empty() || full == "no-gloss" {
                continue;
            }
            let label: String = shorten_gloss(&full).chars().take(90).collect();
            for piece in full.split([',', ';']) {
                let Some(word) = english_item(piece, &pos) else { continue };
                let key = (word.to_lowercase(), pos.clone());
                let at = *self.reverse_index.entry(key.clone()).or_insert_with(|| {
                    self.reverse.push(Entry { head: word.clone(), pos: pos.clone(), ..Default::default() });
                    self.reverse_heads.insert(key.0.clone());
                    self.reverse.len() - 1
                });
                let entry = &mut self.reverse[at];
                let total: usize = entry.senses.iter().map(|s| s.words.len()).sum();
                if total >= 20 {
                    continue;
                }
                let s = match entry.senses.iter().position(|s| s.gloss == label) {
                    Some(s) => s,
                    None if entry.senses.len() < 12 => {
                        entry.senses.push(Sense { gloss: label.clone(), ..Default::default() });
                        entry.senses.len() - 1
                    }
                    None => continue,
                };
                if !entry.senses[s].words.iter().any(|w| w.word == head) && entry.senses[s].words.len() < 8 {
                    entry.senses[s].words.push(Word { word: head.clone(), note: note.clone() });
                }
            }
        }
    }

    /// A line of the "English words with translations" file: the entries that have translations into `lang_code`. For
    /// English words the reverse index already has, it also collects their inflected forms (so "houses" finds "house").
    pub fn feed_translations(&mut self, line: &str, lang_code: &str) {
        self.lines += 1;
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            self.skipped_lines += 1;
            return;
        };
        if str_of(&v, "lang_code") != "en" {
            return;
        }
        let head = str_of(&v, "word").trim().to_string();
        let pos = str_of(&v, "pos").to_string();
        if head.is_empty() || skipped("en", &pos) {
            return;
        }
        let lower = head.to_lowercase();
        if self.reverse_heads.contains(&lower) {
            let forms = forms_of(&v, &head);
            if !forms.is_empty() {
                self.english_forms.entry(lower).or_default().extend(forms);
            }
        }
        // "went: past tense of go" leads to "go".
        for sense in v.get("senses").and_then(Value::as_array).into_iter().flatten() {
            for target in sense.get("form_of").and_then(Value::as_array).into_iter().flatten() {
                let base = str_of(target, "word").trim().to_lowercase();
                if self.reverse_heads.contains(&base) && base != head.to_lowercase() {
                    self.english_links.push((head.clone(), base));
                }
            }
        }
        // Group the translations by the sense they belong to, in file order.
        let mut groups: Vec<Sense> = Vec::new();
        for t in v.get("translations").and_then(Value::as_array).into_iter().flatten() {
            if str_of(t, "lang_code") != lang_code && str_of(t, "code") != lang_code {
                continue;
            }
            let word = str_of(t, "word").trim();
            let tags = strings(t.get("tags"));
            if word.is_empty() || word.len() > 60 || tags.iter().any(|x| is_noise_tag(x)) {
                continue;
            }
            let label = str_of(t, "sense").trim().to_string();
            let note = gender_note(&tags);
            let at = groups.iter().position(|g| g.gloss == label).unwrap_or_else(|| {
                groups.push(Sense { gloss: label.clone(), ..Default::default() });
                groups.len() - 1
            });
            if !groups[at].words.iter().any(|w| w.word == word) && groups[at].words.len() < 8 {
                groups[at].words.push(Word { word: word.to_string(), note });
            }
        }
        if groups.is_empty() {
            return;
        }
        groups.truncate(12);
        let entry = Entry { grammar: grammar_of(&v, &head), examples: Vec::new(), head: head.clone(), pos, senses: groups };
        let forms = forms_of(&v, &head);
        self.entries.push((entry, forms));
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Write the dictionary file for headwords in `source` with translations in `target`.
    pub fn finish(mut self, source: &str, target: &str) -> Writer {
        // The English words from the German glosses join the entries the English file gave, or stand next to them.
        let mut reverse = std::mem::take(&mut self.reverse);
        let mut by_key: HashMap<(String, String), usize> = HashMap::new();
        for (i, (e, _)) in self.entries.iter().enumerate() {
            by_key.insert((e.head.to_lowercase(), e.pos.clone()), i);
        }
        for entry in reverse.drain(..) {
            let key = (entry.head.to_lowercase(), entry.pos.clone());
            let forms = self.english_forms.get(&key.0).cloned().unwrap_or_default();
            match by_key.get(&key) {
                Some(&at) => {
                    let existing = &mut self.entries[at].0;
                    for sense in entry.senses {
                        let new_words: Vec<Word> = sense.words.into_iter().filter(|w| !existing.senses.iter().any(|s| s.words.iter().any(|x| x.word == w.word))).collect();
                        if !new_words.is_empty() && existing.senses.len() < 14 {
                            existing.senses.push(Sense { gloss: sense.gloss, words: new_words, tags: Vec::new() });
                        }
                    }
                }
                None => {
                    by_key.insert(key, self.entries.len());
                    self.entries.push((entry, forms));
                }
            }
        }
        self.links.extend(std::mem::take(&mut self.english_links));
        let mut writer = Writer::new(source, target);
        let mut by_head: HashMap<String, Vec<u32>> = HashMap::new();
        let mut ids: Vec<u32> = Vec::new();
        for (entry, _) in &self.entries {
            let id = writer.add_entry(entry);
            by_head.entry(entry.head.clone()).or_default().push(id);
            ids.push(id);
        }
        for ((_, forms), id) in self.entries.iter().zip(&ids) {
            for form in forms {
                writer.add_key(form, *id, true);
            }
        }
        // "Häuser: plural of Haus" leads to every entry called Haus.
        for (word, base) in &self.links {
            for id in by_head.get(base).into_iter().flatten() {
                writer.add_key(word, *id, true);
            }
        }
        writer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::Dict;
    use crate::lookup::Match;

    const HAUS: &str = r#"{"word":"Haus","lang_code":"de","pos":"noun","head_templates":[{"name":"de-noun","expansion":"Haus n (strong, genitive Hauses, plural Häuser)"}],"forms":[{"form":"Hauses","tags":["genitive"]},{"form":"Häuser","tags":["plural"]},{"form":"strong","tags":["table-tags"]},{"form":"de-ndecl","tags":["inflection-template"]},{"form":"das Haus","tags":["nominative"]}],"senses":[{"glosses":["house","building"],"tags":["neuter"],"examples":[{"text":"Das Haus ist alt.","english":"The house is old.","type":"example"},{"text":"Ein Zitat.","english":"A quote.","type":"quotation"}]},{"glosses":["no-gloss"]}]}"#;
    const HAEUSER: &str = r#"{"word":"Häuser","lang_code":"de","pos":"noun","senses":[{"glosses":["plural of Haus"],"form_of":[{"word":"Haus"}],"tags":["form-of","plural"]}]}"#;
    const GEHEN: &str = r#"{"word":"gehen","lang_code":"de","pos":"verb","head_templates":[{"expansion":"gehen (class 7 strong, third-person singular present geht, past tense ging, past participle gegangen, auxiliary sein)"}],"forms":[{"form":"ging","tags":["past"]},{"form":"gegangen","tags":["participle","past"]},{"form":"ich gehe","tags":["present"]}],"senses":[{"glosses":["to go","to walk"]}]}"#;
    const ENGLISH_WORD: &str = r#"{"word":"Hello","lang_code":"en","pos":"noun","senses":[{"glosses":["x"]}]}"#;
    const CAT: &str = r#"{"word":"cat","lang_code":"en","pos":"noun","head_templates":[{"expansion":"cat (plural cats)"}],"forms":[{"form":"cats","tags":["plural"]}],"translations":[{"lang_code":"de","word":"Katze","sense":"domestic species","tags":["feminine"]},{"lang_code":"de","word":"Kater","sense":"domestic species","tags":["masculine"]},{"lang_code":"de","word":"Mieze","sense":"domestic species","tags":["Alemannic-German"]},{"lang_code":"fr","word":"chat","sense":"domestic species"},{"lang_code":"de","word":"Raubkatze","sense":"feline","tags":[]}]}"#;

    #[test]
    fn german_entries_keep_glosses_grammar_forms_and_examples() {
        let mut c = Collector::new();
        for line in [HAUS, HAEUSER, GEHEN, ENGLISH_WORD, "not json"] {
            c.feed_glosses(line, "de");
        }
        assert_eq!((c.entry_count(), c.skipped_lines), (2, 1), "an inflection entry and an English word are no entries");
        let dict = Dict::from_bytes(c.finish("de", "en").finish()).unwrap();
        let haus = &dict.lookup("Haus").unwrap().hits[0].entry;
        assert_eq!(haus.grammar, "Haus n (strong, genitive Hauses, plural Häuser)");
        assert_eq!(haus.senses.iter().map(|s| s.gloss.as_str()).collect::<Vec<_>>(), ["building"], "the last gloss of a nested list; no-gloss dropped");
        assert_eq!(haus.examples.len(), 1, "quotations are not examples");
        assert_eq!(haus.examples[0].translation, "The house is old.");
    }

    #[test]
    fn inflected_forms_find_the_base_form_through_tables_and_inflection_entries() {
        let mut c = Collector::new();
        for line in [HAUS, HAEUSER, GEHEN] {
            c.feed_glosses(line, "de");
        }
        let dict = Dict::from_bytes(c.finish("de", "en").finish()).unwrap();
        let via = |w: &str| dict.lookup(w).unwrap().hits.iter().map(|h| (h.entry.head.clone(), h.matched)).collect::<Vec<_>>();
        assert_eq!(via("Häuser"), vec![("Haus".to_string(), Match::Form)]);
        assert_eq!(via("Hauses"), vec![("Haus".to_string(), Match::Form)]);
        assert_eq!(via("ging"), vec![("gehen".to_string(), Match::Form)]);
        assert_eq!(via("gegangen"), vec![("gehen".to_string(), Match::Form)]);
        assert!(via("strong").is_empty() && via("de-ndecl").is_empty(), "table metadata is not a word");
        assert!(via("ich gehe").is_empty(), "phrases are not keys");
    }

    #[test]
    fn long_head_lines_and_glosses_are_cut_where_a_reader_can_still_use_them() {
        let long = "Haus n (strong, genitive Hauses, plural Häuser, diminutive Häuschen n or Häuslein n or Häuselein n or Häusle n or Häusel n or Häus'l n)";
        assert_eq!(shorten_grammar(long), "Haus n (strong, genitive Hauses, plural Häuser)");
        assert_eq!(shorten_grammar("cat (plural cats)"), "cat (plural cats)");
        let gloss = "to be going; to be all right [with dative for someone and predicate adjective in a particular way] (idiomatically translated by English be doing or similar, with the dative object as the subject)";
        let short = shorten_gloss(gloss);
        assert!(short.chars().count() <= 152 && short.ends_with('\u{2026}') && short.starts_with("to be going"), "{short}");
        assert_eq!(shorten_gloss("house"), "house");
    }

    #[test]
    fn gender_and_english_words_are_read_from_german_glosses() {
        assert_eq!(gender_from_grammar("Haus", "Haus n (strong, genitive Hauses, plural Häuser)"), "n");
        assert_eq!(gender_from_grammar("Teller", "Teller m (strong, genitive Tellers)"), "m");
        assert_eq!(gender_from_grammar("Katze", "Katze f (genitive Katze, plural Katzen)"), "f");
        assert_eq!(gender_from_grammar("gehen", "gehen (class 7 strong)"), "");
        assert_eq!(english_item("to astonish", "verb").as_deref(), Some("astonish"));
        assert_eq!(english_item("a house", "noun").as_deref(), Some("house"));
        assert_eq!(english_item("a municipality of Styria", "name"), None, "an explanation is not a word");
        assert_eq!(english_item("house (building)", "noun"), None);
    }

    #[test]
    fn the_reverse_direction_comes_from_german_glosses_and_the_english_file_gives_forms() {
        let mut c = Collector::new();
        for line in [r#"{"word":"Haus","lang_code":"de","pos":"noun","head_templates":[{"expansion":"Haus n (strong, genitive Hauses, plural Häuser)"}],"senses":[{"glosses":["house, building"]}]}"#, GEHEN, r#"{"word":"Gebäude","lang_code":"de","pos":"noun","head_templates":[{"expansion":"Gebäude n (strong, genitive Gebäudes, plural Gebäude)"}],"senses":[{"glosses":["building, house"]}]}"#] {
            c.feed_reverse(line, "de");
        }
        // The English file: the plural of "house", "went" as a form of "go", and an entry that has translations.
        for line in [
            r#"{"word":"house","lang_code":"en","pos":"noun","forms":[{"form":"houses","tags":["plural"]}],"senses":[{"glosses":["a building"]}]}"#,
            r#"{"word":"went","lang_code":"en","pos":"verb","senses":[{"glosses":["past of go"],"form_of":[{"word":"go"}]}]}"#,
            CAT,
        ] {
            c.feed_translations(line, "de");
        }
        let dict = Dict::from_bytes(c.finish("en", "de").finish()).unwrap();
        let house = dict.lookup("house").unwrap().hits.into_iter().find(|h| h.entry.pos == "noun").unwrap().entry;
        let words: Vec<(&str, &str)> = house.senses.iter().flat_map(|s| s.words.iter().map(|w| (w.word.as_str(), w.note.as_str()))).collect();
        assert!(words.contains(&("Haus", "n")) && words.contains(&("Gebäude", "n")), "{words:?}");
        assert_eq!(dict.lookup("houses").unwrap().hits[0].entry.head, "house");
        // "to go" has no entry in this sample, but "to walk" does: the verb items lose their "to".
        assert_eq!(dict.lookup("walk").unwrap().hits[0].entry.senses[0].words[0].word, "gehen");
        // Entries from the English file keep their own sense groups and are not duplicated.
        assert_eq!(dict.lookup("cat").unwrap().hits.iter().filter(|h| h.entry.pos == "noun").count(), 1);
    }

    #[test]
    fn english_entries_list_their_german_translations_by_sense_without_dialect_words() {
        let mut c = Collector::new();
        c.feed_translations(CAT, "de");
        c.feed_translations(HAUS, "de");
        assert_eq!(c.entry_count(), 1);
        let dict = Dict::from_bytes(c.finish("en", "de").finish()).unwrap();
        let cat = &dict.lookup("cat").unwrap().hits[0].entry;
        assert_eq!(cat.senses.len(), 2);
        assert_eq!(cat.senses[0].gloss, "domestic species");
        assert_eq!(cat.senses[0].words.iter().map(|w| (w.word.as_str(), w.note.as_str())).collect::<Vec<_>>(), [("Katze", "f"), ("Kater", "m")]);
        assert_eq!(cat.senses[1].words[0].word, "Raubkatze");
        assert_eq!(dict.lookup("cats").unwrap().hits[0].entry.head, "cat");
        assert_eq!(cat.short_translations(3), ["Katze", "Kater", "Raubkatze"]);
    }
}
