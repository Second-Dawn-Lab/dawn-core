//! `models/catalog.json`: the models Second Dawn apps can install and the languages built from them.
//!
//! English is the hub. A model serves some source languages and some target languages
//! (`sources` x `targets`); a plain Opus-MT model serves one pair, a multilingual one many.
//! A language lists the models it needs, and several languages can list the same model: it is
//! downloaded once and kept as long as any installed language needs it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub version: u32,
    pub hub: String,
    pub models: Vec<Model>,
    pub languages: Vec<Language>,
    /// Offline dictionaries (packs of type "dictionary"): one pack per language pair, both directions inside.
    #[serde(default)]
    pub dictionaries: Vec<Dictionary>,
}

/// A dictionary pack for a language pair. Downloaded, verified and extracted like a model, but kept apart from the
/// translation models (own folder and list), so removing a language never touches it and the other way round.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Dictionary {
    /// `en-de`: the two language codes, in alphabetical order.
    pub id: String,
    pub languages: Vec<String>,
    pub urls: Vec<String>,
    pub size_bytes: u64,
    pub sha256: String,
    /// The license of the data ("CC-BY-SA-4.0"): the pack stays under it.
    pub license: String,
    /// Who the data is from, to show wherever the dictionary is used.
    pub attribution: String,
    /// Where to read more or find the original entries.
    pub source_url: String,
}

impl Dictionary {
    /// The pack as a downloadable archive, for the shared download and install code.
    pub fn as_model(&self) -> Model {
        Model {
            id: self.id.clone(),
            sources: self.languages.clone(),
            targets: self.languages.clone(),
            urls: self.urls.clone(),
            size_bytes: self.size_bytes,
            sha256: self.sha256.clone(),
            license: self.license.clone(),
            attribution: self.attribution.clone(),
            prefix: None,
            target_tokens: BTreeMap::new(),
        }
    }

    pub fn covers(&self, a: &str, b: &str) -> bool {
        a != b && self.languages.iter().any(|l| l == a) && self.languages.iter().any(|l| l == b)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Language {
    pub code: String,
    pub name: String,
    pub native_name: String,
    /// Translation quality is known to be unreliable; the UI shows a "Beta" badge.
    #[serde(default)]
    pub beta: bool,
    /// Ids of the models this language needs. It counts as installed when all of them are.
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub id: String,
    /// Languages the model reads.
    pub sources: Vec<String>,
    /// Languages the model writes.
    pub targets: Vec<String>,
    /// Where to download the archive, tried in order; every download is verified against `sha256`.
    /// `https://` or `file://`. A mirror (e.g. GitHub Releases) is added by appending another url.
    pub urls: Vec<String>,
    pub size_bytes: u64,
    pub sha256: String,
    pub license: String,
    pub attribution: String,
    /// Token put in front of every source sentence of a single-target model, e.g. `>>ara<<`.
    #[serde(default)]
    pub prefix: Option<String>,
    /// For models with several targets: the token that selects each one, e.g. `"ur": ">>urd<<"`.
    #[serde(default)]
    pub target_tokens: BTreeMap<String, String>,
}

impl Model {
    /// Can this model translate `from -> to`?
    pub fn serves(&self, from: &str, to: &str) -> bool {
        from != to && self.sources.iter().any(|s| s == from) && self.targets.iter().any(|t| t == to)
    }

    /// The language token to put in front of source sentences when translating into `to`.
    pub fn target_token(&self, to: &str) -> Option<&str> {
        self.target_tokens.get(to).map(String::as_str).or(self.prefix.as_deref())
    }

    /// How many directions it covers; a smaller number means a more specialized model.
    pub fn breadth(&self) -> usize {
        self.sources.len() * self.targets.len()
    }
}

impl Default for Catalog {
    fn default() -> Self {
        Self { version: 2, hub: "en".into(), models: vec![], languages: vec![], dictionaries: vec![] }
    }
}

impl Catalog {
    /// The catalog shipped with the app (`models/catalog.json`).
    pub fn bundled() -> Self {
        Self::from_json(include_str!("../../../models/catalog.json")).unwrap_or_default()
    }

    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }

    pub fn language(&self, code: &str) -> Option<&Language> {
        self.languages.iter().find(|l| l.code == code)
    }

    pub fn dictionary(&self, id: &str) -> Option<&Dictionary> {
        self.dictionaries.iter().find(|d| d.id == id)
    }

    /// The dictionary that has both languages (in either order).
    pub fn dictionary_for(&self, a: &str, b: &str) -> Option<&Dictionary> {
        self.dictionaries.iter().find(|d| d.covers(a, b))
    }

    pub fn model(&self, id: &str) -> Option<&Model> {
        self.models.iter().find(|m| m.id == id)
    }

    /// The models a language needs (ids that are not in the catalog are skipped).
    pub fn language_models(&self, code: &str) -> Vec<&Model> {
        self.language(code).map(|l| l.models.iter().filter_map(|id| self.model(id)).collect()).unwrap_or_default()
    }

    /// Models that can translate `from -> to`, most specialized first.
    pub fn models_serving(&self, from: &str, to: &str) -> Vec<&Model> {
        let mut found: Vec<&Model> = self.models.iter().filter(|m| m.serves(from, to)).collect();
        found.sort_by_key(|m| m.breadth());
        found
    }

    /// The first language that lists this model (used to name what to repair).
    pub fn language_for_model(&self, id: &str) -> Option<&Language> {
        self.languages.iter().find(|l| l.models.iter().any(|m| m == id))
    }

    /// Download size of all of a language's models, ignoring what is already installed.
    pub fn total_size(&self, code: &str) -> u64 {
        self.language_models(code).iter().map(|m| m.size_bytes).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "version": 2, "hub": "en",
      "models": [
        { "id": "en-de", "sources": ["en"], "targets": ["de"], "urls": ["file:///x/en-de.zip"],
          "sizeBytes": 100, "sha256": "aa", "license": "CC-BY-4.0", "attribution": "Helsinki-NLP / OPUS-MT" },
        { "id": "de-en", "sources": ["de"], "targets": ["en"], "urls": ["https://example.org/de-en.zip"],
          "sizeBytes": 50, "sha256": "bb", "license": "Apache-2.0", "attribution": "Helsinki-NLP / OPUS-MT" },
        { "id": "inc", "sources": ["en", "de"], "targets": ["ur", "hi"], "urls": ["file:///x/inc.zip"],
          "sizeBytes": 250, "sha256": "cc", "license": "Apache-2.0", "attribution": "Helsinki-NLP / OPUS-MT",
          "targetTokens": { "ur": ">>urd<<", "hi": ">>hin<<" } },
        { "id": "inc-en", "sources": ["ur", "hi"], "targets": ["en"], "urls": ["file:///x/inc-en.zip"],
          "sizeBytes": 250, "sha256": "dd", "license": "Apache-2.0", "attribution": "Helsinki-NLP / OPUS-MT" }
      ],
      "languages": [
        { "code": "de", "name": "German", "nativeName": "Deutsch", "models": ["en-de", "de-en"] },
        { "code": "ur", "name": "Urdu", "nativeName": "اردو", "beta": true, "models": ["inc", "inc-en"] },
        { "code": "hi", "name": "Hindi", "nativeName": "हिन्दी", "beta": true, "models": ["inc", "inc-en"] }
      ]
    }"#;

    #[test]
    fn parses_shared_model_catalog() {
        let catalog = Catalog::from_json(SAMPLE).unwrap();
        assert_eq!(catalog.hub, "en");
        assert_eq!(catalog.language("de").unwrap().native_name, "Deutsch");
        assert_eq!(catalog.total_size("de"), 150);
        // Urdu and Hindi list the same two models.
        assert_eq!(catalog.language("ur").unwrap().models, catalog.language("hi").unwrap().models);
        assert_eq!(catalog.total_size("ur"), 500);
        assert!(!catalog.language("de").unwrap().beta);
        assert!(catalog.language("ur").unwrap().beta);
    }

    #[test]
    fn models_serve_every_pair_of_their_sources_and_targets() {
        let catalog = Catalog::from_json(SAMPLE).unwrap();
        let inc = catalog.model("inc").unwrap();
        for (from, to) in [("en", "ur"), ("de", "ur"), ("en", "hi"), ("de", "hi")] {
            assert!(inc.serves(from, to), "{from}->{to}");
        }
        assert!(!inc.serves("ur", "en"));
        assert!(!inc.serves("en", "en"));
        assert!(!inc.serves("fr", "ur"));
    }

    #[test]
    fn finds_models_by_direction_most_specialized_first() {
        let catalog = Catalog::from_json(SAMPLE).unwrap();
        assert_eq!(catalog.models_serving("de", "ur").iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["inc"]);
        assert_eq!(catalog.models_serving("en", "de")[0].id, "en-de");
        assert!(catalog.models_serving("de", "fr").is_empty());
    }

    #[test]
    fn target_tokens_pick_the_language_and_fall_back_to_the_prefix() {
        let catalog = Catalog::from_json(SAMPLE).unwrap();
        let inc = catalog.model("inc").unwrap();
        assert_eq!(inc.target_token("ur"), Some(">>urd<<"));
        assert_eq!(inc.target_token("hi"), Some(">>hin<<"));
        assert_eq!(catalog.model("en-de").unwrap().target_token("de"), None);

        let single = Model { prefix: Some(">>ara<<".into()), ..catalog.model("en-de").unwrap().clone() };
        assert_eq!(single.target_token("ar"), Some(">>ara<<"));
    }

    #[test]
    fn names_the_language_to_repair_for_a_model() {
        let catalog = Catalog::from_json(SAMPLE).unwrap();
        assert_eq!(catalog.language_for_model("de-en").unwrap().code, "de");
        assert_eq!(catalog.language_for_model("inc").unwrap().code, "ur");
        assert!(catalog.language_for_model("nope").is_none());
    }

    #[test]
    fn bundled_catalog_is_consistent() {
        let catalog = Catalog::from_json(include_str!("../../../models/catalog.json")).unwrap();
        assert_eq!(catalog.hub, "en");
        for lang in &catalog.languages {
            assert!(!lang.models.is_empty(), "{} has no models", lang.code);
            for id in &lang.models {
                assert!(catalog.model(id).is_some(), "{} lists unknown model {id}", lang.code);
            }
            // Every language can be translated to and from English.
            assert!(!catalog.models_serving("en", &lang.code).is_empty(), "nothing translates en->{}", lang.code);
            assert!(!catalog.models_serving(&lang.code, "en").is_empty(), "nothing translates {}->en", lang.code);
        }
        for m in &catalog.models {
            assert!(!m.sources.is_empty() && !m.targets.is_empty(), "{} serves nothing", m.id);
        }
    }

    /// A url that points at "main" (or any moving ref) could return a different file than the sha256
    /// describes. Bundled urls must be https and, on Hugging Face, pinned to a 40-digit commit hash.
    #[test]
    fn bundled_catalog_urls_are_https_and_pinned_to_a_commit() {
        let catalog = Catalog::bundled();
        assert!(!catalog.models.is_empty());
        for m in &catalog.models {
            assert!(!m.urls.is_empty(), "{} has no url", m.id);
            for url in &m.urls {
                assert!(url.starts_with("https://"), "{}: {url} is not https", m.id);
                assert!(!url.contains("/resolve/main/") && !url.contains("/releases/latest/"), "{}: {url} is not pinned", m.id);
                if let Some(rest) = url.strip_prefix("https://huggingface.co/") {
                    let commit = rest.split("/resolve/").nth(1).and_then(|r| r.split('/').next()).unwrap_or("");
                    assert!(commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()), "{}: {url} lacks a commit hash", m.id);
                    assert!(url.ends_with(&format!("/{}.zip", m.id)), "{}: {url} names another file", m.id);
                }
            }
        }
    }

    #[test]
    fn dictionaries_cover_a_pair_in_either_order_and_are_not_translation_models() {
        let catalog = Catalog::bundled();
        let en_de = catalog.dictionary_for("en", "de").expect("the German-English dictionary");
        assert_eq!(en_de.id, "de-en");
        assert!(en_de.covers("en", "de") && !en_de.covers("en", "fr") && !en_de.covers("en", "en"));
        assert!(en_de.license.starts_with("CC-BY-SA"), "{}", en_de.license);
        assert!(en_de.attribution.contains("Wiktionary") && en_de.attribution.contains("kaikki.org"), "{}", en_de.attribution);
        // A dictionary is never listed among the models a language needs.
        assert!(catalog.model(&en_de.id).is_none());
        let model = en_de.as_model();
        assert_eq!((model.sha256.as_str(), model.size_bytes), (en_de.sha256.as_str(), en_de.size_bytes));
    }

    /// Same rule as for the translation packs: https, and on Hugging Face a commit hash, never a moving ref.
    #[test]
    fn dictionary_urls_are_https_and_pinned_to_a_commit() {
        let catalog = Catalog::bundled();
        for d in &catalog.dictionaries {
            assert!(!d.urls.is_empty() && d.sha256.len() == 64, "{} is not published", d.id);
            for url in &d.urls {
                assert!(url.starts_with("https://") && !url.contains("/resolve/main/"), "{}: {url}", d.id);
                if let Some(rest) = url.strip_prefix("https://huggingface.co/") {
                    let commit = rest.split("/resolve/").nth(1).and_then(|r| r.split('/').next()).unwrap_or("");
                    assert!(commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()), "{}: {url} lacks a commit hash", d.id);
                    assert!(url.ends_with(&format!("/{}.zip", d.id)) || url.ends_with(&format!("/dictionary-{}.zip", d.id)), "{}: {url} names another file", d.id);
                }
            }
        }
    }

    #[test]
    fn urdu_japanese_and_the_indo_aryan_languages_are_beta() {
        let catalog = Catalog::bundled();
        for code in ["ur", "ja", "hi", "pa", "bn"] {
            assert!(catalog.language(code).is_some_and(|l| l.beta), "{code} should be beta");
        }
        for code in ["de", "fr", "es"] {
            assert!(catalog.language(code).is_some_and(|l| !l.beta), "{code} should not be beta");
        }
    }
}
