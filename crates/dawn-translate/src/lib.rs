//! Translation engines behind a single trait.
//!
//! * [`Ct2Engine`]: the real engine. Opus-MT models run through CTranslate2, with English as
//!   the hub: pairs without a direct model are translated `from -> en -> to`.
//! * [`StubEngine`]: a fake for tests and UI work.
//! * [`detect_language`]: language detection over all known languages.

mod alternatives;
mod ct2;
mod detect;
mod glossary;
mod lang_token;
mod router;
mod split;

use std::{thread, time::Duration};

use anyhow::{bail, Result};

pub use alternatives::{clean_candidates, first_word_pieces, word_bounds, word_bounds_all, Alternative, Candidate};
pub use glossary::{placeholder, protect, restore, Glossary, GlossaryEntry, Protected, Restored};
pub use ct2::{default_threads, Ct2Engine, Ct2Options, Ct2Runner, StoreResolver};
pub use detect::{detect_in_context, detect_language, is_short, Detection};
pub use lang_token::{language_tokens, split_language_token, LangTokenizer};
pub use split::{join_document, split_document, split_sentences};
pub use router::{ModelSpec, Pass, Plan, Resolver, Router, Runner, Segment, TranslateError, TranslateOptions, Translation, Update};

/// A translation backend.
pub trait TranslationEngine {
    /// Translate `text` from language `from` to language `to`.
    fn translate(&self, text: &str, from: &str, to: &str) -> Result<String>;

    /// All `(from, to)` language pairs this engine can handle.
    fn supported_pairs(&self) -> Vec<(String, String)>;
}

/// Fake engine for tests and UI development: echoes the text tagged with the target language.
#[derive(Debug, Clone)]
pub struct StubEngine {
    pub delay: Duration,
    pub languages: Vec<String>,
}

impl StubEngine {
    pub fn new(languages: Vec<String>) -> Self {
        Self { delay: Duration::from_millis(150), languages }
    }
}

impl TranslationEngine for StubEngine {
    fn translate(&self, text: &str, from: &str, to: &str) -> Result<String> {
        if !self.languages.iter().any(|l| l == from) || !self.languages.iter().any(|l| l == to) {
            bail!("unsupported pair {from} -> {to}");
        }
        thread::sleep(self.delay);
        Ok(format!("[{to}] {text}"))
    }

    fn supported_pairs(&self) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        for a in &self.languages {
            for b in &self.languages {
                if a != b {
                    pairs.push((a.clone(), b.clone()));
                }
            }
        }
        pairs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_tags_output() {
        let engine = StubEngine::new(vec!["en".into(), "de".into()]);
        assert_eq!(engine.translate("Hi", "en", "de").unwrap(), "[de] Hi");
    }
}
