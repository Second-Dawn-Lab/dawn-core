//! A glossary: terms that must come out in a given way ("Owl" -> "Eule", "Kernel" stays "Kernel").
//!
//! Translation models cannot be told to use a word, so the glossary works around them:
//!
//! 1. **Protect.** Before a sentence is translated, every glossary term found in it is replaced by
//!    a placeholder token ([`placeholder`]) that the model copies through unchanged.
//! 2. **Restore.** After the translation, each placeholder is replaced by the target term.
//!
//! A placeholder the model dropped or mangled is not guessed at: the sentence is reported as
//! "not applied" and keeps the model's own wording. The limit of this technique is grammar: the
//! target term is inserted as written, so it is never declined, pluralized or re-cased to fit the
//! sentence ("Eule" stays "Eule" where German would say "Eulen" or "der Eule"). See docs/glossary.md.

use serde::{Deserialize, Serialize};

fn yes() -> bool {
    true
}

/// One term: `source` in language `from` is written `target` in language `to`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryEntry {
    #[serde(default)]
    pub id: u64,
    pub from: String,
    pub to: String,
    pub source: String,
    pub target: String,
    /// "Owl" does not match "owl".
    #[serde(default)]
    pub match_case: bool,
    /// "cat" does not match inside "category".
    #[serde(default = "yes")]
    pub whole_word: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Glossary {
    /// The global switch.
    pub enabled: bool,
    pub entries: Vec<GlossaryEntry>,
}

impl Default for Glossary {
    fn default() -> Self {
        Self { enabled: true, entries: Vec::new() }
    }
}

impl Glossary {
    /// The entries that apply when translating `from` -> `to` (none when the glossary is off).
    pub fn for_pair(&self, from: &str, to: &str) -> Vec<&GlossaryEntry> {
        if !self.enabled {
            return Vec::new();
        }
        self.entries.iter().filter(|e| e.enabled && e.from == from && e.to == to && !e.source.trim().is_empty()).collect()
    }

    /// Give new entries (id 0) the next free id.
    pub fn assign_ids(&mut self) {
        let mut next = self.entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        for e in &mut self.entries {
            if e.id == 0 {
                e.id = next;
                next += 1;
            }
        }
    }

    /// `source,target,from,to,match_case,whole_word,enabled` with a header line.
    pub fn to_csv(&self) -> String {
        let mut out = String::from("source,target,from,to,match_case,whole_word,enabled\n");
        for e in &self.entries {
            let cols = [csv_quote(&e.source), csv_quote(&e.target), csv_quote(&e.from), csv_quote(&e.to), e.match_case.to_string(), e.whole_word.to_string(), e.enabled.to_string()];
            out.push_str(&cols.join(","));
            out.push('\n');
        }
        out
    }

    /// Read entries from CSV. The header is optional; columns are source, target, from, to and then
    /// the optional flags (`true/false`, `1/0`, `yes/no`). Rows without a source or target are skipped.
    /// `default_pair` fills `from`/`to` when the file has only two columns.
    pub fn parse_csv(text: &str, default_pair: Option<(&str, &str)>) -> (Vec<GlossaryEntry>, usize) {
        let rows = parse_csv_rows(text.trim_start_matches('\u{feff}'));
        let mut entries = Vec::new();
        let mut skipped = 0;
        for (n, row) in rows.iter().enumerate() {
            if row.iter().all(|c| c.trim().is_empty()) {
                continue;
            }
            if n == 0 && row.first().is_some_and(|c| c.trim().eq_ignore_ascii_case("source")) {
                continue;
            }
            let get = |i: usize| row.get(i).map(|c| c.trim()).filter(|c| !c.is_empty());
            let (source, target) = (get(0), get(1));
            let (from, to) = match (get(2), get(3)) {
                (Some(f), Some(t)) => (f.to_string(), t.to_string()),
                _ => match default_pair {
                    Some((f, t)) => (f.to_string(), t.to_string()),
                    None => {
                        skipped += 1;
                        continue;
                    }
                },
            };
            let (Some(source), Some(target)) = (source, target) else {
                skipped += 1;
                continue;
            };
            let flag = |i: usize, default: bool| get(i).map_or(default, |v| matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "y" | "x"));
            entries.push(GlossaryEntry {
                id: 0,
                from,
                to,
                source: source.to_string(),
                target: target.to_string(),
                match_case: flag(4, false),
                whole_word: flag(5, true),
                enabled: flag(6, true),
            });
        }
        (entries, skipped)
    }
}

fn csv_quote(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// Comma separated, or semicolon separated when the first line has no comma (Excel in many
/// locales saves CSV that way).
fn parse_csv_rows(text: &str) -> Vec<Vec<String>> {
    let first_line = text.lines().next().unwrap_or("");
    let delimiter = if !first_line.contains(',') && first_line.contains(';') { ';' } else { ',' };
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            (true, '"') => quoted = false,
            (true, _) => field.push(c),
            (false, '"') if field.is_empty() => quoted = true,
            (false, c) if c == delimiter => row.push(std::mem::take(&mut field)),
            (false, '\r') => {}
            (false, '\n') => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            (false, _) => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

// --- protecting and restoring terms -------------------------------------------------------

/// The token that stands in for the `index`-th term (1-based) of a sentence while it is translated.
///
/// It has to be copied through by the model, survive a pivot through English and not be taken for
/// a word to translate. See docs/glossary.md for the shapes that were tried.
pub fn placeholder(index: usize) -> String {
    format!("Zq{index}x")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Protected {
    /// The sentence with every matched term replaced by its placeholder.
    pub text: String,
    /// The sentence with every matched term replaced by its target term, written as it should come
    /// out. Models copy a word they do not know and often get its article right (see `restore`).
    pub inline: String,
    /// `(placeholder, target term)` in sentence order.
    pub terms: Vec<(String, String)>,
    /// The text of the sentence each term replaced, in the same order ("astonished").
    pub sources: Vec<String>,
}

impl Protected {
    pub fn unchanged(text: &str) -> Self {
        Self { text: text.to_string(), inline: text.to_string(), terms: Vec::new(), sources: Vec::new() }
    }

    /// Do all target terms appear in `output` (ignoring case)? Used to accept the inline form.
    pub fn all_terms_in(&self, output: &str) -> bool {
        let lower = output.to_lowercase();
        self.terms.iter().all(|(_, target)| lower.contains(&target.to_lowercase()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restored {
    pub text: String,
    /// Terms put in.
    pub applied: usize,
    /// Terms whose placeholder did not come back (the model dropped or changed it).
    pub missing: usize,
}

#[derive(Debug, PartialEq)]
struct Match {
    start: usize,
    end: usize,
    entry: usize,
}

fn chars_equal(a: char, b: char, match_case: bool) -> bool {
    a == b || (!match_case && a.to_lowercase().eq(b.to_lowercase()))
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Non-overlapping matches of `entries` in `chars`, leftmost first, longest first on ties.
fn find_matches(chars: &[char], entries: &[&GlossaryEntry]) -> Vec<Match> {
    let terms: Vec<Vec<char>> = entries.iter().map(|e| e.source.trim().chars().collect()).collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let mut best: Option<(usize, usize)> = None; // (entry, length)
        for (k, term) in terms.iter().enumerate() {
            let len = term.len();
            if len == 0 || i + len > chars.len() {
                continue;
            }
            let e = entries[k];
            if !term.iter().zip(&chars[i..i + len]).all(|(a, b)| chars_equal(*a, *b, e.match_case)) {
                continue;
            }
            if e.whole_word {
                let before_ok = i == 0 || !is_word_char(chars[i - 1]) || !is_word_char(chars[i]);
                let after_ok = i + len == chars.len() || !is_word_char(chars[i + len]) || !is_word_char(chars[i + len - 1]);
                if !before_ok || !after_ok {
                    continue;
                }
            }
            if best.is_none_or(|(_, l)| len > l) {
                best = Some((k, len));
            }
        }
        match best {
            Some((entry, len)) => {
                found.push(Match { start: i, end: i + len, entry });
                i += len;
            }
            None => i += 1,
        }
    }
    found
}

/// What the glossary does to `sentence`: a fingerprint of the entries that match it (their terms, targets
/// and options). A translation or a pinned choice made under one fingerprint is not valid under another,
/// so adding, editing, disabling or switching a term re-translates the sentences it touches and no others.
/// 0 means the glossary does nothing here.
pub fn signature(sentence: &str, entries: &[&GlossaryEntry]) -> u64 {
    use std::hash::{Hash, Hasher};
    let chars: Vec<char> = sentence.chars().collect();
    let matches = find_matches(&chars, entries);
    if matches.is_empty() {
        return 0;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for m in &matches {
        let e = entries[m.entry];
        (m.start, &e.source, &e.target, e.match_case, e.whole_word).hash(&mut hasher);
    }
    hasher.finish().max(1)
}

/// Replace every glossary term in `sentence` by a placeholder.
pub fn protect(sentence: &str, entries: &[&GlossaryEntry]) -> Protected {
    if entries.is_empty() {
        return Protected::unchanged(sentence);
    }
    let chars: Vec<char> = sentence.chars().collect();
    let matches = find_matches(&chars, entries);
    if matches.is_empty() {
        return Protected::unchanged(sentence);
    }
    let mut text = String::with_capacity(sentence.len());
    let mut inline = String::with_capacity(sentence.len());
    let mut terms = Vec::new();
    let mut sources = Vec::new();
    let mut cursor = 0;
    for (n, m) in matches.iter().enumerate() {
        text.extend(&chars[cursor..m.start]);
        inline.extend(&chars[cursor..m.start]);
        sources.push(chars[m.start..m.end].iter().collect::<String>());
        let token = placeholder(n + 1);
        let target = entries[m.entry].target.trim().to_string();
        text.push_str(&token);
        inline.push_str(&target);
        terms.push((token, target));
        cursor = m.end;
    }
    text.extend(&chars[cursor..]);
    inline.extend(&chars[cursor..]);
    Protected { text, inline, terms, sources }
}

/// The byte range of the first case-insensitive occurrence of `needle` in `hay`.
fn find_ignoring_case(hay: &str, needle: &str) -> Option<(usize, usize)> {
    let needle: Vec<char> = needle.chars().collect();
    if needle.is_empty() {
        return None;
    }
    let hay_chars: Vec<(usize, char)> = hay.char_indices().collect();
    'start: for i in 0..hay_chars.len() {
        if i + needle.len() > hay_chars.len() {
            return None;
        }
        for (k, c) in needle.iter().enumerate() {
            if !chars_equal(hay_chars[i + k].1, *c, false) {
                continue 'start;
            }
        }
        let end = hay_chars.get(i + needle.len()).map_or(hay.len(), |(b, _)| *b);
        return Some((hay_chars[i].0, end));
    }
    None
}

/// The inline form lets the model keep the target term, but a model capitalizes a word it takes for a name
/// ("Xsdcv"). A glossary term is inserted as written, so each target found in `output` is rewritten exactly.
pub fn exact_terms(output: &str, protected: &Protected) -> String {
    let mut text = output.to_string();
    let mut from = 0;
    for (_, target) in &protected.terms {
        if let Some((start, end)) = find_ignoring_case(&text[from..], target) {
            text.replace_range(from + start..from + end, target);
            from += start + target.len();
        }
    }
    text
}

/// Put the target terms where the placeholders came back.
pub fn restore(output: &str, protected: &Protected) -> Restored {
    let mut text = output.to_string();
    let (mut applied, mut missing) = (0, 0);
    for (token, target) in &protected.terms {
        match text.find(token.as_str()) {
            Some(at) => {
                text.replace_range(at..at + token.len(), target);
                applied += 1;
            }
            None => missing += 1,
        }
    }
    // A placeholder that was duplicated by the model must not leak into the text.
    for (token, target) in &protected.terms {
        while let Some(at) = text.find(token.as_str()) {
            text.replace_range(at..at + token.len(), target);
        }
    }
    Restored { text, applied, missing }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(source: &str, target: &str) -> GlossaryEntry {
        GlossaryEntry { id: 0, from: "en".into(), to: "de".into(), source: source.into(), target: target.into(), match_case: false, whole_word: true, enabled: true }
    }

    fn protect_with(sentence: &str, entries: &[GlossaryEntry]) -> Protected {
        let refs: Vec<&GlossaryEntry> = entries.iter().collect();
        protect(sentence, &refs)
    }

    #[test]
    fn a_term_is_replaced_by_a_placeholder_and_restored() {
        let p = protect_with("The owl sleeps.", &[entry("owl", "Eule")]);
        assert_eq!(p.text, format!("The {} sleeps.", placeholder(1)));
        let r = restore(&format!("Die {} schläft.", placeholder(1)), &p);
        assert_eq!(r, Restored { text: "Die Eule schläft.".into(), applied: 1, missing: 0 });
    }

    #[test]
    fn matching_ignores_case_unless_asked() {
        let mut e = entry("Owl", "Eule");
        assert_ne!(protect_with("An OWL and an owl.", std::slice::from_ref(&e)).text, "An OWL and an owl.");
        assert_eq!(protect_with("An OWL and an owl.", std::slice::from_ref(&e)).terms.len(), 2);
        e.match_case = true;
        let p = protect_with("An OWL and an owl and an Owl.", &[e]);
        assert_eq!(p.terms.len(), 1);
        assert!(p.text.starts_with("An OWL and an owl and an Zq1x"));
    }

    #[test]
    fn whole_word_does_not_match_inside_other_words() {
        let e = entry("cat", "Katze");
        assert_eq!(protect_with("The category has a cat.", std::slice::from_ref(&e)).terms.len(), 1);
        let mut inner = e;
        inner.whole_word = false;
        assert_eq!(protect_with("The category has a cat.", &[inner]).terms.len(), 2);
    }

    #[test]
    fn punctuation_next_to_a_term_is_a_word_boundary() {
        let p = protect_with("(owl), owl-like, \"owl\".", &[entry("owl", "Eule")]);
        assert_eq!(p.terms.len(), 3);
    }

    #[test]
    fn the_longest_overlapping_term_wins() {
        let p = protect_with("The snowy owl sleeps.", &[entry("owl", "Eule"), entry("snowy owl", "Schneeeule")]);
        assert_eq!(p.terms, vec![(placeholder(1), "Schneeeule".to_string())]);
        assert_eq!(p.text, format!("The {} sleeps.", placeholder(1)));
    }

    #[test]
    fn several_terms_are_numbered_in_sentence_order() {
        let p = protect_with("The owl met a fox and another owl.", &[entry("owl", "Eule"), entry("fox", "Fuchs")]);
        assert_eq!(p.terms.iter().map(|t| t.1.as_str()).collect::<Vec<_>>(), ["Eule", "Fuchs", "Eule"]);
        let r = restore(&format!("Die {} traf einen {} und eine andere {}.", placeholder(1), placeholder(2), placeholder(3)), &p);
        assert_eq!(r.text, "Die Eule traf einen Fuchs und eine andere Eule.");
        assert_eq!(r.applied, 3);
    }

    #[test]
    fn the_signature_changes_with_the_matching_terms_and_only_with_those() {
        let owl = entry("owl", "Eule");
        let fox = entry("fox", "Fuchs");
        let sig = |s: &str, e: &[&GlossaryEntry]| signature(s, e);
        assert_eq!(sig("The cat sleeps.", &[&owl]), 0, "no term, no effect");
        let a = sig("The owl sleeps.", &[&owl]);
        assert_ne!(a, 0);
        assert_eq!(a, sig("The owl sleeps.", &[&owl, &fox]), "an unrelated term does not change it");
        assert_ne!(a, sig("The owl sleeps.", &[&entry("owl", "Kauz")]), "another target does");
        assert_ne!(a, sig("The owl sleeps.", &[]), "removing the term does");
        let mut cased = owl.clone();
        cased.match_case = true;
        assert_ne!(a, sig("The owl sleeps.", &[&cased]), "an option does");
    }

    #[test]
    fn a_term_the_model_capitalized_is_written_as_in_the_glossary() {
        let p = protect_with("I'm astonished and the owl too.", &[entry("astonished", "xsdcv"), entry("owl", "Eule")]);
        assert_eq!(exact_terms("Ich bin Xsdcv und die eule auch.", &p), "Ich bin xsdcv und die Eule auch.");
        assert_eq!(exact_terms("Nichts zu tun.", &p), "Nichts zu tun.");
    }

    #[test]
    fn the_matched_source_text_is_kept_for_the_tooltip() {
        let p = protect_with("An Owl and a fox.", &[entry("owl", "Eule"), entry("fox", "Fuchs")]);
        assert_eq!(p.sources, vec!["Owl".to_string(), "fox".to_string()]);
    }

    #[test]
    fn the_inline_form_has_the_target_terms_written_in() {
        let p = protect_with("The owl met a fox.", &[entry("owl", "Eule"), entry("fox", "Fuchs")]);
        assert_eq!(p.inline, "The Eule met a Fuchs.");
        assert!(p.all_terms_in("Die eule traf einen Fuchs."), "case is ignored");
        assert!(!p.all_terms_in("Die Eule traf einen Hund."));
        assert!(Protected::unchanged("x").all_terms_in("anything"));
    }

    #[test]
    fn a_placeholder_the_model_lost_is_counted_as_missing() {
        let p = protect_with("The owl sleeps.", &[entry("owl", "Eule")]);
        let r = restore("Die Eulen schlafen.", &p);
        assert_eq!((r.text.as_str(), r.applied, r.missing), ("Die Eulen schlafen.", 0, 1));
    }

    #[test]
    fn a_term_without_a_match_leaves_the_sentence_alone() {
        let p = protect_with("Nothing to see.", &[entry("owl", "Eule")]);
        assert_eq!(p, Protected::unchanged("Nothing to see."));
    }

    #[test]
    fn multi_word_and_non_latin_terms_work() {
        let p = protect_with("Ich lese das schwarze Brett.", &[GlossaryEntry { from: "de".into(), to: "en".into(), ..entry("schwarze Brett", "bulletin board") }]);
        assert_eq!(p.terms.len(), 1);
        let urdu = protect_with("یہ ایک اُلو ہے", &[entry("اُلو", "Eule")]);
        assert_eq!(urdu.terms.len(), 1);
    }

    #[test]
    fn only_enabled_entries_of_the_pair_apply_and_the_global_switch_wins() {
        let mut g = Glossary::default();
        g.entries = vec![entry("owl", "Eule"), GlossaryEntry { enabled: false, ..entry("fox", "Fuchs") }, GlossaryEntry { to: "fr".into(), ..entry("owl", "hibou") }];
        assert_eq!(g.for_pair("en", "de").len(), 1);
        assert_eq!(g.for_pair("en", "fr").len(), 1);
        g.enabled = false;
        assert!(g.for_pair("en", "de").is_empty());
    }

    #[test]
    fn csv_round_trips_with_quotes_commas_and_flags() {
        let mut g = Glossary::default();
        g.entries = vec![
            GlossaryEntry { match_case: true, ..entry("owl, snowy", "Schnee\"eule\"") },
            GlossaryEntry { whole_word: false, enabled: false, ..entry("fox", "Fuchs") },
        ];
        let csv = g.to_csv();
        let (back, skipped) = Glossary::parse_csv(&csv, None);
        assert_eq!(skipped, 0);
        assert_eq!(back, g.entries);
    }

    #[test]
    fn csv_without_a_header_or_pair_columns_uses_the_default_pair() {
        let (entries, skipped) = Glossary::parse_csv("\u{feff}owl,Eule\r\n\r\nfox,Fuchs\r\nbroken\r\n", Some(("en", "de")));
        assert_eq!(entries.len(), 2);
        assert_eq!(skipped, 1);
        assert_eq!((entries[0].from.as_str(), entries[0].to.as_str(), entries[0].whole_word, entries[0].enabled), ("en", "de", true, true));
        let (none, skipped) = Glossary::parse_csv("owl,Eule\n", None);
        assert!(none.is_empty());
        assert_eq!(skipped, 1);
    }

    #[test]
    fn semicolon_separated_csv_from_excel_is_read() {
        let (entries, _) = Glossary::parse_csv("source;target\nowl;Eule, die\n", Some(("en", "de")));
        assert_eq!((entries[0].source.as_str(), entries[0].target.as_str()), ("owl", "Eule, die"));
    }

    #[test]
    fn ids_are_assigned_once() {
        let mut g = Glossary::default();
        g.entries = vec![GlossaryEntry { id: 5, ..entry("a", "b") }, entry("c", "d"), entry("e", "f")];
        g.assign_ids();
        assert_eq!(g.entries.iter().map(|e| e.id).collect::<Vec<_>>(), [5, 6, 7]);
    }
}
