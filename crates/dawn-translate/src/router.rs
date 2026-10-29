//! Picks the models for a language pair and runs the text through them.
//!
//! English is the hub: a pair is translated directly if a model for it exists, otherwise
//! `from -> en -> to`. The router knows nothing about CTranslate2; it works with any
//! [`Resolver`] (where models are) and [`Runner`] (how to run one).
//!
//! Translation streams: sentences go through the models in small batches and every finished
//! batch is reported through a callback. With two passes (the default for the app) the text
//! is first translated with a fast beam, so something appears almost at once, and then with
//! the normal beam, replacing the draft sentence by sentence. A per-sentence cache (model,
//! beam, sentence) means only sentences that changed are translated again while typing.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::split::{join_document, split_document};

/// An installed model ready to be loaded.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelSpec {
    pub dir: PathBuf,
    /// Token put in front of every source sentence (multi-target models).
    pub prefix: Option<String>,
}

pub trait Resolver: Send + Sync {
    /// An installed model translating exactly `from -> to`.
    fn resolve(&self, from: &str, to: &str) -> Option<ModelSpec>;
}

pub trait Runner: Send + Sync {
    /// Translate a batch of sentences with one model at the given beam size.
    fn run(&self, model: &ModelSpec, sentences: &[String], beam: usize) -> Result<Vec<String>, TranslateError>;
}

#[derive(Debug)]
pub enum TranslateError {
    /// No installed model covers this language (it must be downloaded first).
    ModelMissing(String),
    /// A model is installed but could not be loaded (corrupt files); repair by re-downloading.
    ModelLoad { model: String, reason: String },
    Cancelled,
    Engine(String),
}

impl TranslateError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::ModelMissing(_) => "pack_missing",
            Self::ModelLoad { .. } => "model_load",
            Self::Cancelled => "cancelled",
            Self::Engine(_) => "engine",
        }
    }
}

impl fmt::Display for TranslateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModelMissing(lang) => write!(f, "{lang}"),
            Self::ModelLoad { model, reason } => write!(f, "could not load model {model}: {reason}"),
            Self::Cancelled => write!(f, "cancelled"),
            Self::Engine(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for TranslateError {}

#[derive(Debug, Clone, PartialEq)]
pub struct Translation {
    pub text: String,
    /// The text went through English (`from -> en -> to`).
    pub pivoted: bool,
}

#[derive(Debug, PartialEq)]
pub enum Plan {
    Same,
    Direct(ModelSpec),
    Pivot(ModelSpec, ModelSpec),
}

/// How a translation is run.
#[derive(Debug, Clone)]
pub struct TranslateOptions {
    /// Sentences per model call; each finished batch is reported.
    pub batch_size: usize,
    /// Beam size of the quick first pass (`None`: a single pass at `beam`).
    pub fast_beam: Option<usize>,
    /// Beam size of the final pass.
    pub beam: usize,
}

impl Default for TranslateOptions {
    /// One beam-2 pass, streamed in batches of four sentences.
    fn default() -> Self {
        Self { batch_size: 4, fast_beam: None, beam: 2 }
    }
}

impl TranslateOptions {
    /// A quick beam-1 draft first, then beam 2 replaces it sentence by sentence (the optional
    /// "two-pass" setting). Roughly doubles the total work, so it is off by default.
    pub fn with_draft() -> Self {
        Self { fast_beam: Some(1), ..Self::default() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pass {
    /// The quick draft.
    Fast,
    /// The final quality pass (also used when there is only one pass).
    Refine,
}

/// A snapshot of the result while it is being produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    /// The translation so far (a prefix during the fast pass, the whole text afterwards).
    pub text: String,
    pub pass: Pass,
    /// Sentences finished in this pass, out of `total`.
    pub done: usize,
    pub total: usize,
    pub pivoted: bool,
    /// A complete first-pass draft is already showing, so this pass replaces it quietly.
    pub draft_shown: bool,
}

impl Update {
    /// The draft is complete: everything is on screen, quality is still being improved.
    pub fn draft_complete(&self) -> bool {
        self.pass == Pass::Fast && self.done == self.total
    }

    /// Nothing more will change.
    pub fn is_final(&self) -> bool {
        self.pass == Pass::Refine && self.done == self.total
    }
}

/// (model folder, target-language token, beam, source sentence): the same multilingual model
/// gives a different translation for each target token.
type CacheKey = (PathBuf, Option<String>, usize, String);

/// Translations by (model, beam, source sentence); oldest entries go first.
struct SentenceCache {
    map: HashMap<CacheKey, String>,
    order: VecDeque<CacheKey>,
    capacity: usize,
}

impl SentenceCache {
    fn new(capacity: usize) -> Self {
        Self { map: HashMap::new(), order: VecDeque::new(), capacity }
    }

    fn get(&self, model: &ModelSpec, beam: usize, sentence: &str) -> Option<String> {
        self.map.get(&(model.dir.clone(), model.prefix.clone(), beam, sentence.to_string())).cloned()
    }

    fn put(&mut self, model: &ModelSpec, beam: usize, sentence: &str, translation: String) {
        let key = (model.dir.clone(), model.prefix.clone(), beam, sentence.to_string());
        if self.map.insert(key.clone(), translation).is_none() {
            self.order.push_back(key);
        }
        while self.order.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
    }
}

pub struct Router<R, N> {
    resolver: R,
    runner: N,
    hub: String,
    cache: Mutex<SentenceCache>,
}

impl<R: Resolver, N: Runner> Router<R, N> {
    pub fn new(resolver: R, runner: N, hub: impl Into<String>) -> Self {
        Self { resolver, runner, hub: hub.into(), cache: Mutex::new(SentenceCache::new(4096)) }
    }

    pub fn runner(&self) -> &N {
        &self.runner
    }

    /// Forget cached sentences (e.g. after a language pack was removed or repaired).
    pub fn clear_cache(&self) {
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = SentenceCache::new(4096);
    }

    pub fn plan(&self, from: &str, to: &str) -> Result<Plan, TranslateError> {
        if from == to {
            return Ok(Plan::Same);
        }
        if let Some(direct) = self.resolver.resolve(from, to) {
            return Ok(Plan::Direct(direct));
        }
        if from == self.hub {
            return Err(TranslateError::ModelMissing(to.to_string()));
        }
        if to == self.hub {
            return Err(TranslateError::ModelMissing(from.to_string()));
        }
        let first = self.resolver.resolve(from, &self.hub).ok_or_else(|| TranslateError::ModelMissing(from.to_string()))?;
        let second = self.resolver.resolve(&self.hub, to).ok_or_else(|| TranslateError::ModelMissing(to.to_string()))?;
        Ok(Plan::Pivot(first, second))
    }

    /// Translate `text` in one quality pass, without reporting progress.
    pub fn translate(&self, text: &str, from: &str, to: &str, cancel: &AtomicBool) -> Result<Translation, TranslateError> {
        let options = TranslateOptions { batch_size: 16, fast_beam: None, beam: 2 };
        self.translate_streaming(text, from, to, cancel, &options, &mut |_| {})
    }

    /// Translate `text`, calling `on_update` after every finished batch.
    ///
    /// `cancel` is checked between batches; setting it returns [`TranslateError::Cancelled`]
    /// (used so the newest keystroke wins).
    pub fn translate_streaming(
        &self,
        text: &str,
        from: &str,
        to: &str,
        cancel: &AtomicBool,
        options: &TranslateOptions,
        on_update: &mut dyn FnMut(Update),
    ) -> Result<Translation, TranslateError> {
        let (stages, pivoted) = match self.plan(from, to)? {
            Plan::Same => return Ok(Translation { text: text.to_string(), pivoted: false }),
            Plan::Direct(model) => (vec![model], false),
            Plan::Pivot(a, b) => (vec![a, b], true),
        };

        let doc = split_document(text, from);
        let sources = doc.sentences();
        let total = sources.len();
        if total == 0 {
            return Ok(Translation { text: text.to_string(), pivoted });
        }
        let batch = options.batch_size.max(1);
        let draft_shown = options.fast_beam.is_some_and(|b| b != options.beam);
        let mut output: Vec<Option<String>> = vec![None; total];
        // Sentences already translated at the final beam (nothing left to refine).
        let mut refined = vec![false; total];

        let snapshot = |output: &[Option<String>]| -> String {
            let ready: Vec<String> = output.iter().map_while(|o| o.clone()).collect();
            join_document(&doc, &ready, to)
        };
        let check = || if cancel.load(Ordering::Relaxed) { Err(TranslateError::Cancelled) } else { Ok(()) };

        // Pass 1: a draft. Sentences already known at the final beam are used as they are.
        if let Some(fast) = options.fast_beam.filter(|b| *b != options.beam) {
            for start in (0..total).step_by(batch) {
                check()?;
                let end = (start + batch).min(total);
                let mut todo = Vec::new();
                for i in start..end {
                    match self.peek(&stages, &sources[i], options.beam) {
                        Some(best) => {
                            output[i] = Some(best);
                            refined[i] = true;
                        }
                        None => todo.push(i),
                    }
                }
                if !todo.is_empty() {
                    let inputs: Vec<String> = todo.iter().map(|&i| sources[i].clone()).collect();
                    let draft = self.run_stages(&stages, &inputs, fast)?;
                    for (i, text) in todo.into_iter().zip(draft) {
                        output[i] = Some(text);
                    }
                }
                on_update(Update { text: snapshot(&output), pass: Pass::Fast, done: end, total, pivoted, draft_shown: false });
            }
        }

        // Pass 2 (or the only pass): the final beam, replacing sentence by sentence.
        let mut done = refined.iter().filter(|r| **r).count();
        let pending: Vec<usize> = (0..total).filter(|i| !refined[*i]).collect();
        if pending.is_empty() {
            on_update(Update { text: snapshot(&output), pass: Pass::Refine, done: total, total, pivoted, draft_shown });
        }
        for group in pending.chunks(batch) {
            check()?;
            let inputs: Vec<String> = group.iter().map(|&i| sources[i].clone()).collect();
            let better = self.run_stages(&stages, &inputs, options.beam)?;
            for (&i, text) in group.iter().zip(better) {
                output[i] = Some(text);
            }
            done += group.len();
            // With a single pass the output is still being filled in order: show a prefix.
            on_update(Update { text: snapshot(&output), pass: Pass::Refine, done, total, pivoted, draft_shown });
        }

        let final_text = snapshot(&output);
        Ok(Translation { text: final_text, pivoted })
    }

    /// The finished translation of one sentence at `beam`, if every stage is already cached.
    fn peek(&self, stages: &[ModelSpec], sentence: &str, beam: usize) -> Option<String> {
        let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let mut current = sentence.to_string();
        for model in stages {
            current = cache.get(model, beam, &current)?;
        }
        Some(current)
    }

    /// Run sentences through every stage, translating only what is not cached.
    fn run_stages(&self, stages: &[ModelSpec], sentences: &[String], beam: usize) -> Result<Vec<String>, TranslateError> {
        let mut current = sentences.to_vec();
        for model in stages {
            current = self.run_cached(model, &current, beam)?;
        }
        Ok(current)
    }

    fn run_cached(&self, model: &ModelSpec, sentences: &[String], beam: usize) -> Result<Vec<String>, TranslateError> {
        let cached: Vec<Option<String>> = {
            let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            sentences.iter().map(|s| cache.get(model, beam, s)).collect()
        };
        let misses: Vec<String> = sentences.iter().zip(&cached).filter(|(_, c)| c.is_none()).map(|(s, _)| s.clone()).collect();
        let mut fresh = if misses.is_empty() { Vec::new() } else { self.runner.run(model, &misses, beam)? }.into_iter();

        let mut out = Vec::with_capacity(sentences.len());
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        for (sentence, hit) in sentences.iter().zip(cached) {
            match hit {
                Some(text) => out.push(text),
                None => {
                    let text = fresh.next().ok_or_else(|| TranslateError::Engine("the engine returned too few sentences".into()))?;
                    cache.put(model, beam, sentence, text.clone());
                    out.push(text);
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Models "installed": a set of (from, to) pairs.
    struct FakeResolver(HashSet<(&'static str, &'static str)>);

    impl Resolver for FakeResolver {
        fn resolve(&self, from: &str, to: &str) -> Option<ModelSpec> {
            self.0
                .iter()
                .find(|(f, t)| *f == from && *t == to)
                .map(|(f, t)| ModelSpec { dir: PathBuf::from(format!("{f}-{t}")), prefix: None })
        }
    }

    /// Tags every sentence with the model and beam that handled it, e.g. `[de-en|b2](Hallo.)`,
    /// and remembers every call as `(beam, sentence count)`.
    #[derive(Default)]
    struct FakeRunner {
        calls: Mutex<Vec<(usize, usize)>>,
    }

    impl Runner for FakeRunner {
        fn run(&self, model: &ModelSpec, sentences: &[String], beam: usize) -> Result<Vec<String>, TranslateError> {
            self.calls.lock().unwrap().push((beam, sentences.len()));
            Ok(sentences.iter().map(|s| format!("[{}|b{beam}]({s})", model.dir.display())).collect())
        }
    }

    fn router(pairs: &[(&'static str, &'static str)]) -> Router<FakeResolver, FakeRunner> {
        Router::new(FakeResolver(pairs.iter().copied().collect()), FakeRunner::default(), "en")
    }

    const ALL: &[(&str, &str)] = &[("en", "de"), ("de", "en"), ("en", "ur"), ("ur", "en")];

    fn no_cancel() -> AtomicBool {
        AtomicBool::new(false)
    }

    fn numbered(n: usize) -> String {
        (1..=n).map(|i| format!("Sentence {i}.")).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn same_language_returns_the_input() {
        let out = router(&[]).translate("Hello.", "de", "de", &no_cancel()).unwrap();
        assert_eq!(out, Translation { text: "Hello.".into(), pivoted: false });
    }

    #[test]
    fn direct_model_is_used_when_available() {
        let out = router(ALL).translate("Hello.", "en", "de", &no_cancel()).unwrap();
        assert_eq!(out.text, "[en-de|b2](Hello.)");
        assert!(!out.pivoted);
    }

    #[test]
    fn missing_pair_pivots_through_english() {
        let out = router(ALL).translate("Hallo.", "de", "ur", &no_cancel()).unwrap();
        assert_eq!(out.text, "[en-ur|b2]([de-en|b2](Hallo.))");
        assert!(out.pivoted);
    }

    #[test]
    fn direct_pair_beats_pivot() {
        let mut pairs = ALL.to_vec();
        pairs.push(("de", "ur"));
        let out = router(&pairs).translate("Hallo.", "de", "ur", &no_cancel()).unwrap();
        assert_eq!(out.text, "[de-ur|b2](Hallo.)");
        assert!(!out.pivoted);
    }

    #[test]
    fn reports_which_language_is_missing() {
        let r = router(&[("en", "de"), ("de", "en")]);
        let c = no_cancel();
        assert!(matches!(r.translate("x", "en", "ur", &c), Err(TranslateError::ModelMissing(l)) if l == "ur"));
        assert!(matches!(r.translate("x", "ur", "en", &c), Err(TranslateError::ModelMissing(l)) if l == "ur"));
        assert!(matches!(r.translate("x", "de", "ur", &c), Err(TranslateError::ModelMissing(l)) if l == "ur"));
        assert!(matches!(r.translate("x", "ur", "de", &c), Err(TranslateError::ModelMissing(l)) if l == "ur"));
    }

    #[test]
    fn keeps_line_structure() {
        let r = router(ALL);
        let text = (0..40).map(|i| format!("Line {i}.")).collect::<Vec<_>>().join("\n");
        let out = r.translate(&text, "en", "de", &no_cancel()).unwrap();
        assert_eq!(out.text.lines().count(), 40);
    }

    #[test]
    fn cancel_stops_before_the_first_batch() {
        let r = router(ALL);
        let err = r.translate("One. Two.", "en", "de", &AtomicBool::new(true)).unwrap_err();
        assert!(matches!(err, TranslateError::Cancelled));
        assert!(r.runner.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn text_without_sentences_is_returned_unchanged() {
        assert_eq!(router(ALL).translate("  \n ", "en", "de", &no_cancel()).unwrap().text, "  \n ");
    }

    #[test]
    fn streams_small_batches_two_passes() {
        let r = router(ALL);
        let mut updates = Vec::new();
        let out = r
            .translate_streaming(&numbered(10), "en", "de", &no_cancel(), &TranslateOptions::with_draft(), &mut |u| updates.push(u))
            .unwrap();

        // Batches of 4: fast pass 4+4+2 at beam 1, then final pass 4+4+2 at beam 2.
        assert_eq!(*r.runner.calls.lock().unwrap(), vec![(1, 4), (1, 4), (1, 2), (2, 4), (2, 4), (2, 2)]);
        let fast: Vec<_> = updates.iter().filter(|u| u.pass == Pass::Fast).map(|u| u.done).collect();
        let refine: Vec<_> = updates.iter().filter(|u| u.pass == Pass::Refine).map(|u| u.done).collect();
        assert_eq!(fast, vec![4, 8, 10]);
        assert_eq!(refine, vec![4, 8, 10]);
        assert!(updates[2].draft_complete());
        assert!(updates.last().unwrap().is_final());

        // The draft grows from the front; the final text uses beam 2 everywhere.
        assert!(updates[0].text.contains("b1") && !updates[0].text.contains("Sentence 5"));
        assert!(updates[2].text.contains("Sentence 10"));
        assert!(!out.text.contains("b1") && out.text.contains("Sentence 10"));
        assert_eq!(out.text, updates.last().unwrap().text);
    }

    #[test]
    fn default_is_one_streamed_beam_2_pass() {
        let r = router(ALL);
        let mut updates = Vec::new();
        let out = r.translate_streaming(&numbered(10), "en", "de", &no_cancel(), &TranslateOptions::default(), &mut |u| updates.push(u)).unwrap();
        assert_eq!(*r.runner.calls.lock().unwrap(), vec![(2, 4), (2, 4), (2, 2)], "no beam-1 draft");
        assert!(updates.iter().all(|u| u.pass == Pass::Refine));
        assert_eq!(updates.iter().map(|u| u.done).collect::<Vec<_>>(), vec![4, 8, 10]);
        // The result grows from the front and every sentence is already final quality.
        assert!(updates[0].text.contains("Sentence 4") && !updates[0].text.contains("Sentence 5"));
        assert!(updates.iter().all(|u| !u.text.contains("b1")));
        assert_eq!(out.text, updates.last().unwrap().text);
    }

    #[test]
    fn updates_say_whether_a_draft_is_showing() {
        let r = router(ALL);
        let mut single = Vec::new();
        r.translate_streaming(&numbered(6), "en", "de", &no_cancel(), &TranslateOptions::default(), &mut |u| single.push(u)).unwrap();
        assert!(single.iter().all(|u| !u.draft_shown), "a single pass fills in front of the user");
        r.clear_cache();
        let mut two = Vec::new();
        r.translate_streaming(&numbered(6), "en", "de", &no_cancel(), &TranslateOptions::with_draft(), &mut |u| two.push(u)).unwrap();
        assert!(two.iter().filter(|u| u.pass == Pass::Refine).all(|u| u.draft_shown));
        assert!(two.iter().filter(|u| u.pass == Pass::Fast).all(|u| !u.draft_shown));
    }

    #[test]
    fn a_second_identical_request_runs_nothing() {
        let r = router(ALL);
        let first = r.translate_streaming(&numbered(6), "en", "de", &no_cancel(), &TranslateOptions::with_draft(), &mut |_| {}).unwrap();
        let calls = r.runner.calls.lock().unwrap().len();
        let mut updates = Vec::new();
        let again = r
            .translate_streaming(&numbered(6), "en", "de", &no_cancel(), &TranslateOptions::with_draft(), &mut |u| updates.push(u))
            .unwrap();
        assert_eq!(r.runner.calls.lock().unwrap().len(), calls, "everything came from the cache");
        assert_eq!(first, again);
        assert!(updates.last().unwrap().is_final());
    }

    #[test]
    fn editing_one_sentence_translates_only_that_one() {
        let r = router(ALL);
        let opts = TranslateOptions::with_draft();
        r.translate_streaming(&numbered(8), "en", "de", &no_cancel(), &opts, &mut |_| {}).unwrap();
        r.runner.calls.lock().unwrap().clear();

        let edited = numbered(8).replace("Sentence 3.", "Something else.");
        let out = r.translate_streaming(&edited, "en", "de", &no_cancel(), &opts, &mut |_| {}).unwrap();
        // One changed sentence: one call at beam 1 (draft), one at beam 2 (final), one sentence each.
        assert_eq!(*r.runner.calls.lock().unwrap(), vec![(1, 1), (2, 1)]);
        assert!(out.text.contains("Something else.") && out.text.contains("Sentence 8."));
        assert!(!out.text.contains("b1"));
    }

    #[test]
    fn cache_is_keyed_by_beam_and_model() {
        let r = router(ALL);
        let single = TranslateOptions { batch_size: 4, fast_beam: None, beam: 1 };
        r.translate_streaming("One.", "en", "de", &no_cancel(), &single, &mut |_| {}).unwrap();
        let other_beam = TranslateOptions { batch_size: 4, fast_beam: None, beam: 2 };
        r.translate_streaming("One.", "en", "de", &no_cancel(), &other_beam, &mut |_| {}).unwrap();
        r.translate_streaming("One.", "en", "ur", &no_cancel(), &other_beam, &mut |_| {}).unwrap();
        assert_eq!(*r.runner.calls.lock().unwrap(), vec![(1, 1), (2, 1), (2, 1)]);
    }

    #[test]
    fn one_model_with_different_target_tokens_does_not_share_cache_entries() {
        // Urdu and Hindi come from the same multilingual model, selected by a token.
        struct TokenResolver;
        impl Resolver for TokenResolver {
            fn resolve(&self, _from: &str, to: &str) -> Option<ModelSpec> {
                Some(ModelSpec { dir: PathBuf::from("inc"), prefix: Some(format!(">>{to}<<")) })
            }
        }
        struct Echo;
        impl Runner for Echo {
            fn run(&self, model: &ModelSpec, sentences: &[String], _beam: usize) -> Result<Vec<String>, TranslateError> {
                Ok(sentences.iter().map(|s| format!("{}{s}", model.prefix.clone().unwrap_or_default())).collect())
            }
        }
        let r = Router::new(TokenResolver, Echo, "en");
        let opts = TranslateOptions::default();
        let ur = r.translate_streaming("Hello.", "de", "ur", &no_cancel(), &opts, &mut |_| {}).unwrap();
        let hi = r.translate_streaming("Hello.", "de", "hi", &no_cancel(), &opts, &mut |_| {}).unwrap();
        assert_eq!(ur.text, ">>ur<<Hello.");
        assert_eq!(hi.text, ">>hi<<Hello.", "must not reuse the Urdu translation");
    }

    #[test]
    fn pivot_stages_are_cached_separately() {
        let r = router(ALL);
        let opts = TranslateOptions { batch_size: 4, fast_beam: None, beam: 2 };
        r.translate_streaming("Hallo.", "de", "ur", &no_cancel(), &opts, &mut |_| {}).unwrap();
        r.runner.calls.lock().unwrap().clear();
        // The first stage (de->en) of that pivot is cached, so translating to English is free.
        let out = r.translate_streaming("Hallo.", "de", "en", &no_cancel(), &opts, &mut |_| {}).unwrap();
        assert!(r.runner.calls.lock().unwrap().is_empty());
        assert_eq!(out.text, "[de-en|b2](Hallo.)");
    }

    #[test]
    fn cancel_between_batches_stops_the_job() {
        let r = router(ALL);
        let cancel = AtomicBool::new(false);
        let mut seen = 0;
        let err = r
            .translate_streaming(&numbered(12), "en", "de", &cancel, &TranslateOptions::with_draft(), &mut |_| {
                seen += 1;
                if seen == 2 {
                    cancel.store(true, Ordering::Relaxed);
                }
            })
            .unwrap_err();
        assert!(matches!(err, TranslateError::Cancelled));
        assert_eq!(r.runner.calls.lock().unwrap().len(), 2, "no work after the cancel");
    }

    #[test]
    fn cache_forgets_old_sentences() {
        let mut cache = SentenceCache::new(2);
        let model = ModelSpec { dir: PathBuf::from("m"), prefix: None };
        for s in ["a", "b", "c"] {
            cache.put(&model, 2, s, s.to_uppercase());
        }
        assert_eq!(cache.get(&model, 2, "a"), None);
        assert_eq!(cache.get(&model, 2, "c").as_deref(), Some("C"));
    }
}
