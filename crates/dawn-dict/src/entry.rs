//! What a dictionary entry holds. Stored as compact JSON inside the pack; the short field names keep the pack small.

use serde::{Deserialize, Serialize};

/// A word of the other language that stands for a sense ("Haus", note "n").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Word {
    #[serde(rename = "w")]
    pub word: String,
    /// Gender or other short note ("m", "f", "n", "pl").
    #[serde(rename = "n", default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// One meaning. `gloss` is the definition (English in the German to English direction, a short sense label in the
/// other); `words` are the equivalents in the other language when the data has them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Sense {
    #[serde(rename = "g", default, skip_serializing_if = "String::is_empty")]
    pub gloss: String,
    #[serde(rename = "w", default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<Word>,
    /// Usage labels ("informal", "figurative").
    #[serde(rename = "t", default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Example {
    #[serde(rename = "x")]
    pub text: String,
    #[serde(rename = "y", default, skip_serializing_if = "String::is_empty")]
    pub translation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Entry {
    #[serde(rename = "h")]
    pub head: String,
    /// "noun", "verb", "adj", ...
    #[serde(rename = "p")]
    pub pos: String,
    /// The head line as Wiktionary writes it: "Haus n (strong, genitive Hauses, plural Häuser)",
    /// "gehen (class 7 strong, third-person singular present geht, past tense ging, past participle gegangen, auxiliary sein)".
    #[serde(rename = "r", default, skip_serializing_if = "String::is_empty")]
    pub grammar: String,
    #[serde(rename = "s", default)]
    pub senses: Vec<Sense>,
    #[serde(rename = "e", default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<Example>,
}

impl Entry {
    /// The equivalents of the first senses, flattened and de-duplicated: a short answer for a narrow popup.
    pub fn short_translations(&self, max: usize) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for sense in &self.senses {
            let candidates: Vec<&str> = if sense.words.is_empty() { vec![sense.gloss.as_str()] } else { sense.words.iter().map(|w| w.word.as_str()).collect() };
            for c in candidates {
                if !c.is_empty() && !out.iter().any(|o| o == c) {
                    out.push(c.to_string());
                }
                if out.len() >= max {
                    return out;
                }
            }
        }
        out
    }
}
