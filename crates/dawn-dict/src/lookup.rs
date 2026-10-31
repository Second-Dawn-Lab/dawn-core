//! Looking a word up: the entries of its headword, then the entries it is an inflected form of ("Häuser" leads to
//! "Haus", "ging" to "gehen"), and when nothing is found, the closest headwords.

use crate::entry::Entry;
use crate::format::{Dict, DictError};

/// How a hit was reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Match {
    /// The word is the entry's headword.
    Head,
    /// The word is an inflected form of the entry's headword (the entry is its base form).
    Form,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub entry: Entry,
    pub matched: Match,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Lookup {
    pub hits: Vec<Hit>,
    /// Closest headwords when there are no hits.
    pub suggestions: Vec<String>,
}

const MAX_HITS: usize = 8;
const MAX_SUGGESTIONS: usize = 5;

/// Letters and digits of the word, without the punctuation that surrounds it in text.
pub fn clean_word(word: &str) -> &str {
    word.trim_matches(|c: char| !(c.is_alphanumeric()))
}

/// Edit distance, giving up above `limit` (returns `limit + 1`).
fn distance(a: &[char], b: &[char], limit: usize) -> usize {
    if a.len().abs_diff(b.len()) > limit {
        return limit + 1;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        let mut best = cur[0];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            let v = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
            best = best.min(v);
            cur.push(v);
        }
        if best > limit {
            return limit + 1;
        }
        prev = cur;
    }
    prev[b.len()]
}

impl Dict {
    /// The entries for `word`: headword matches first (an exact spelling before another capitalization), then the
    /// entries whose inflected forms include it.
    pub fn lookup(&self, word: &str) -> Result<Lookup, DictError> {
        let word = clean_word(word);
        if word.is_empty() {
            return Ok(Lookup::default());
        }
        let mut found: Vec<(u32, bool)> = self.postings(word);
        // German "ß" is written "ss" in Swiss spelling and by keyboards without it.
        if found.is_empty() && word.contains("ss") {
            found = self.postings(&word.replace("ss", "\u{df}"));
        }
        let mut hits: Vec<Hit> = Vec::new();
        for (id, form) in found {
            if hits.len() >= MAX_HITS * 2 {
                break;
            }
            hits.push(Hit { entry: self.entry(id)?, matched: if form { Match::Form } else { Match::Head } });
        }
        // Head before form; inside the heads the exact spelling first.
        // Among equals the fuller entry first: "went" is "go" before it is an obscure word that lists "went" as a form.
        let weight = |h: &Hit| h.entry.senses.iter().map(|s| 1 + s.words.len()).sum::<usize>();
        hits.sort_by_key(|h| (h.matched == Match::Form, h.entry.head != word, usize::MAX - weight(h)));
        hits.truncate(MAX_HITS);
        let suggestions = if hits.is_empty() { self.suggest(word)? } else { Vec::new() };
        Ok(Lookup { hits, suggestions })
    }

    /// Headwords that start like `word`, else those within two edits, nearest first.
    pub fn suggest(&self, word: &str) -> Result<Vec<String>, DictError> {
        let key = crate::format::normalize(word);
        let mut keys: Vec<String> = Vec::new();
        let start = self.lower_bound(key.as_bytes());
        let mut i = start;
        while i < self.key_count() && keys.len() < MAX_SUGGESTIONS * 3 {
            let k = self.key_str(i);
            if !k.starts_with(&key) {
                break;
            }
            keys.push(k);
            i += 1;
        }
        if keys.is_empty() {
            // Same first letter, similar spelling.
            let first: String = key.chars().take(1).collect();
            let word_chars: Vec<char> = key.chars().collect();
            let mut scored: Vec<(usize, String)> = Vec::new();
            let mut i = self.lower_bound(first.as_bytes());
            let limit = if word_chars.len() > 5 { 2 } else { 1 };
            while i < self.key_count() {
                let k = self.key_str(i);
                if !k.starts_with(&first) {
                    break;
                }
                let chars: Vec<char> = k.chars().collect();
                let d = distance(&word_chars, &chars, limit);
                if d <= limit {
                    scored.push((d, k));
                }
                i += 1;
            }
            scored.sort_by(|a, b| a.cmp(b));
            keys = scored.into_iter().map(|(_, k)| k).take(MAX_SUGGESTIONS * 3).collect();
        }
        // Show them as the dictionary writes them (with its capital letters), only headwords, once each.
        let mut out: Vec<String> = Vec::new();
        for k in keys {
            for (id, form) in self.postings(&k) {
                if form {
                    continue;
                }
                let head = self.entry(id)?.head;
                if !out.contains(&head) {
                    out.push(head);
                }
                break;
            }
            if out.len() >= MAX_SUGGESTIONS {
                break;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::{Sense, Word};
    use crate::format::Writer;

    fn entry(head: &str, pos: &str, translation: &str) -> Entry {
        Entry { head: head.into(), pos: pos.into(), senses: vec![Sense { gloss: translation.into(), words: vec![Word { word: translation.into(), note: String::new() }], tags: vec![] }], ..Default::default() }
    }

    fn sample() -> Dict {
        let mut w = Writer::new("de", "en");
        let erstaunen = w.add_entry(&entry("erstaunen", "verb", "to astonish"));
        for f in ["erstaunt", "erstaunte", "erstaunst"] {
            w.add_key(f, erstaunen, true);
        }
        w.add_entry(&entry("erstaunt", "adj", "astonished"));
        let haus = w.add_entry(&entry("Haus", "noun", "house"));
        w.add_key("Häuser", haus, true);
        w.add_key("Hauses", haus, true);
        let gehen = w.add_entry(&entry("gehen", "verb", "to go"));
        w.add_key("ging", gehen, true);
        w.add_key("gegangen", gehen, true);
        w.add_entry(&entry("Straße", "noun", "street"));
        w.add_entry(&entry("Hausaufgabe", "noun", "homework"));
        Dict::from_bytes(w.finish()).unwrap()
    }

    #[test]
    fn inflected_words_lead_to_their_base_forms() {
        let d = sample();
        let heads = |w: &str| d.lookup(w).unwrap().hits.iter().map(|h| (h.entry.head.clone(), h.matched)).collect::<Vec<_>>();
        assert_eq!(heads("Häuser"), vec![("Haus".to_string(), Match::Form)]);
        assert_eq!(heads("ging"), vec![("gehen".to_string(), Match::Form)]);
        // "erstaunt" is an adjective of its own and a form of the verb: the headword comes first.
        assert_eq!(heads("erstaunt"), vec![("erstaunt".to_string(), Match::Head), ("erstaunen".to_string(), Match::Form)]);
    }

    #[test]
    fn punctuation_and_capitals_around_a_word_do_not_matter() {
        let d = sample();
        assert_eq!(d.lookup("„Haus“,").unwrap().hits[0].entry.head, "Haus");
        assert_eq!(d.lookup("HAUS").unwrap().hits[0].entry.head, "Haus");
        assert_eq!(d.lookup("Strasse").unwrap().hits[0].entry.head, "Straße");
        assert!(d.lookup("   ").unwrap().hits.is_empty());
    }

    #[test]
    fn an_unknown_word_gets_the_closest_headwords() {
        let d = sample();
        let miss = d.lookup("Hausaufgab").unwrap();
        assert!(miss.hits.is_empty());
        assert_eq!(miss.suggestions, vec!["Hausaufgabe".to_string()]);
        let typo = d.lookup("Hauss").unwrap();
        assert_eq!(typo.suggestions.first().map(String::as_str), Some("Haus"), "{:?}", typo.suggestions);
        assert!(d.lookup("Zzzzzz").unwrap().suggestions.is_empty());
    }

    #[test]
    fn edit_distance_gives_up_above_the_limit() {
        let c = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(distance(&c("haus"), &c("haus"), 2), 0);
        assert_eq!(distance(&c("haus"), &c("hauss"), 2), 1);
        assert_eq!(distance(&c("haus"), &c("katze"), 2), 3);
    }
}
