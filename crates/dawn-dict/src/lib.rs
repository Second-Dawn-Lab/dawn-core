//! Offline dictionary: a compact read-only file per direction ("German to English"), lookup of a word or of
//! the base form of an inflected word, and the reader that builds those files from Wiktionary extracts
//! published by kaikki.org (see `docs/dictionary.md` for the data, its license and how a pack is made).

pub mod entry;
pub mod format;
pub mod kaikki;
pub mod lookup;

pub use entry::{Entry, Example, Sense, Word};
pub use format::{normalize, Dict, DictError, Writer};
pub use lookup::{clean_word, Hit, Lookup, Match};
