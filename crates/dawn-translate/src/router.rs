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
use std::sync::{Arc, Mutex, RwLock};

use crate::alternatives::Alternative;
use crate::glossary::{exact_terms, protect, restore, signature, Glossary, Protected};
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

    /// Up to `limit` words for the position where the word at UTF-16 offset `word_start` of `sentence` (the
    /// translation of `source`) stands, the current word first. Engines that cannot do this say so, and the
    /// app hides the feature.
    fn alternatives(&self, _model: &ModelSpec, _source: &str, _sentence: &str, _word_start: usize, _current_word: &str, _limit: usize) -> Result<Vec<Alternative>, TranslateError> {
        Err(TranslateError::Engine("this engine cannot suggest alternatives".into()))
    }

    /// The n best whole translations of `source` (the best first), as the model ranks them.
    fn sentence_alternatives(&self, _model: &ModelSpec, _source: &str, _current: &str, _count: usize) -> Result<Vec<String>, TranslateError> {
        Err(TranslateError::Engine("this engine cannot suggest sentence alternatives".into()))
    }
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
    /// The translated sentences in order, for word alternatives (empty when there is nothing to pick from).
    pub segments: Vec<Segment>,
    /// How many glossary terms were put into the text.
    pub glossary_hits: usize,
    /// Where glossary terms stand in the text (for highlighting).
    pub glossary_terms: Vec<TermHit>,
}

impl Translation {
    fn plain(text: &str, pivoted: bool) -> Self {
        Self { text: text.to_string(), pivoted, segments: Vec::new(), glossary_hits: 0, glossary_terms: Vec::new() }
    }
}

/// A glossary term in the translated text.
#[derive(Debug, Clone, PartialEq)]
pub struct TermHit {
    /// Position of the term in [`Translation::text`], in UTF-16 units.
    pub start: usize,
    pub end: usize,
    /// What the term replaced in the source sentence ("astonished").
    pub source: String,
    /// The term as inserted ("xsdcv").
    pub target: String,
}

/// One translated sentence and where it sits in the final text.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// The source sentence.
    pub source: String,
    /// What the last model stage read: the source, or the English sentence of a pivoted translation.
    pub input: String,
    /// The finished translation of the sentence.
    pub target: String,
    /// Position of `target` in [`Translation::text`], in UTF-16 units (what JavaScript indexes by).
    pub start: usize,
    pub end: usize,
    /// The sentence was chosen by the user from the alternatives and stays as it is.
    pub pinned: bool,
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

/// Sentences the user picked from the word alternatives: (from, to, source sentence) -> (input of the
/// last stage, translation). They win over the model and survive re-translation while the user types,
/// until the source sentence itself changes.
#[derive(Default)]
struct Pins {
    /// (from, to, source sentence) -> (input of the last stage, translation, glossary signature of the sentence).
    map: HashMap<(String, String, String), (String, String, u64)>,
    order: VecDeque<(String, String, String)>,
}

const MAX_PINS: usize = 512;

pub struct Router<R, N> {
    resolver: R,
    runner: N,
    hub: String,
    cache: Mutex<SentenceCache>,
    pins: Mutex<Pins>,
    glossary: RwLock<Arc<Glossary>>,
}

impl<R: Resolver, N: Runner> Router<R, N> {
    pub fn new(resolver: R, runner: N, hub: impl Into<String>) -> Self {
        Self {
            resolver,
            runner,
            hub: hub.into(),
            cache: Mutex::new(SentenceCache::new(4096)),
            pins: Mutex::new(Pins::default()),
            glossary: RwLock::new(Arc::new(Glossary::default())),
        }
    }

    /// Use this glossary from now on. Cached sentences stay valid: they are keyed by the protected
    /// sentence, so a changed term simply gives a new key.
    pub fn set_glossary(&self, glossary: Glossary) {
        *self.glossary.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(glossary);
    }

    /// Remember the user's pick for a sentence (see [`Pins`]).
    pub fn pin(&self, from: &str, to: &str, source: &str, input: &str, translation: &str) {
        // The pick was made under the glossary as it is now; it is only valid while that stays true for this sentence.
        let glossary = self.glossary.read().unwrap_or_else(|e| e.into_inner()).clone();
        let sig = signature(source, &glossary.for_pair(from, to));
        let key = (from.to_string(), to.to_string(), source.to_string());
        let mut pins = self.pins.lock().unwrap_or_else(|e| e.into_inner());
        if pins.map.insert(key.clone(), (input.to_string(), translation.to_string(), sig)).is_none() {
            pins.order.push_back(key);
        }
        while pins.order.len() > MAX_PINS {
            if let Some(oldest) = pins.order.pop_front() {
                pins.map.remove(&oldest);
            }
        }
    }

    /// The user's pick for a sentence, if it is still valid: when the glossary now treats the sentence
    /// differently than when it was picked (a term added, edited, disabled or switched), the pick is dropped.
    fn pinned(&self, from: &str, to: &str, source: &str, glossary_signature: u64) -> Option<(String, String)> {
        let mut pins = self.pins.lock().unwrap_or_else(|e| e.into_inner());
        let key = (from.to_string(), to.to_string(), source.to_string());
        match pins.map.get(&key) {
            Some((input, translation, sig)) if *sig == glossary_signature => Some((input.clone(), translation.clone())),
            Some(_) => {
                pins.map.remove(&key);
                pins.order.retain(|k| k != &key);
                None
            }
            None => None,
        }
    }

    /// Up to `count` whole-sentence alternatives for the translation of `input`, different from `current`.
    pub fn sentence_alternatives(&self, from: &str, to: &str, input: &str, current: &str, count: usize) -> Result<Vec<String>, TranslateError> {
        let last_from = match self.plan(from, to)? {
            Plan::Same => return Ok(Vec::new()),
            Plan::Direct(_) => from.to_string(),
            Plan::Pivot(..) => self.hub.clone(),
        };
        let model = self.resolver.resolve(&last_from, to).ok_or_else(|| TranslateError::ModelMissing(to.to_string()))?;
        self.runner.sentence_alternatives(&model, input, current, count)
    }

    /// Alternatives for the word that follows `prefix` in the translation of `input`, where `input` is
    /// what the last model stage read (`Segment::input`).
    pub fn alternatives(&self, from: &str, to: &str, input: &str, sentence: &str, word_start: usize, current_word: &str, limit: usize) -> Result<Vec<Alternative>, TranslateError> {
        let last_from = match self.plan(from, to)? {
            Plan::Same => return Ok(Vec::new()),
            Plan::Direct(_) => from.to_string(),
            Plan::Pivot(..) => self.hub.clone(),
        };
        let model = self.resolver.resolve(&last_from, to).ok_or_else(|| TranslateError::ModelMissing(to.to_string()))?;
        self.runner.alternatives(&model, input, sentence, word_start, current_word, limit)
    }

    pub fn runner(&self) -> &N {
        &self.runner
    }

    /// Forget cached sentences (e.g. after a language pack was removed or repaired).
    pub fn clear_cache(&self) {
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = SentenceCache::new(4096);
        *self.pins.lock().unwrap_or_else(|e| e.into_inner()) = Pins::default();
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
            Plan::Same => return Ok(Translation::plain(text, false)),
            Plan::Direct(model) => (vec![model], false),
            Plan::Pivot(a, b) => (vec![a, b], true),
        };

        let doc = split_document(text, from);
        let sources = doc.sentences();
        let total = sources.len();
        if total == 0 {
            return Ok(Translation::plain(text, pivoted));
        }
        let batch = options.batch_size.max(1);
        let draft_shown = options.fast_beam.is_some_and(|b| b != options.beam);
        let mut output: Vec<Option<String>> = vec![None; total];
        // Sentences already translated at the final beam (nothing left to refine).
        let mut refined = vec![false; total];
        // What the last stage read, per sentence (for word alternatives).
        let mut last_inputs: Vec<String> = sources.clone();
        let mut pinned = vec![false; total];

        // Glossary terms are swapped for placeholders before translation and put back afterwards.
        let glossary = self.glossary.read().unwrap_or_else(|e| e.into_inner()).clone();
        let entries = glossary.for_pair(from, to);
        let protected: Vec<Protected> = sources.iter().map(|s| protect(s, &entries)).collect();
        // Writing the target term into the English sentence works best, but only for a direct model:
        // after a pivot the intermediate English would carry (say) Urdu script into the second model.
        let inline_ok = stages.len() == 1;

        // Sentences the user picked from the alternatives are not translated again.
        for i in 0..total {
            if let Some((input, translation)) = self.pinned(from, to, &sources[i], signature(&sources[i], &entries)) {
                output[i] = Some(translation);
                last_inputs[i] = input;
                refined[i] = true;
                pinned[i] = true;
            }
        }

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
                    if refined[i] {
                        continue;
                    }
                    match self.peek_protected(&stages, &protected[i], options.beam, inline_ok) {
                        Some((last_input, best)) => {
                            output[i] = Some(best);
                            last_inputs[i] = last_input;
                            refined[i] = true;
                        }
                        None => todo.push(i),
                    }
                }
                if !todo.is_empty() {
                    let items: Vec<&Protected> = todo.iter().map(|&i| &protected[i]).collect();
                    let draft = self.run_protected(&stages, &items, fast, inline_ok)?;
                    for (i, (last_input, text)) in todo.into_iter().zip(draft) {
                        output[i] = Some(text);
                        last_inputs[i] = last_input;
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
            let items: Vec<&Protected> = group.iter().map(|&i| &protected[i]).collect();
            let better = self.run_protected(&stages, &items, options.beam, inline_ok)?;
            for (&i, (last_input, text)) in group.iter().zip(better) {
                output[i] = Some(text);
                last_inputs[i] = last_input;
            }
            done += group.len();
            // With a single pass the output is still being filled in order: show a prefix.
            on_update(Update { text: snapshot(&output), pass: Pass::Refine, done, total, pivoted, draft_shown });
        }

        let final_text = snapshot(&output);
        let segments = build_segments(&final_text, &sources, &last_inputs, &output, &pinned);
        // Counted once per sentence in the final pass; the draft pass would count them twice.
        let glossary_hits = (0..total).map(|i| restore_count(&protected[i], output[i].as_deref().unwrap_or(""))).sum();
        let glossary_terms = term_hits(&segments, &protected, &output);
        Ok(Translation { text: final_text, pivoted, segments, glossary_hits, glossary_terms })
    }

    /// The finished translation of one sentence at `beam`, if every stage is already cached, with
    /// what the last stage read.
    fn peek(&self, stages: &[ModelSpec], sentence: &str, beam: usize) -> Option<(String, String)> {
        let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let mut current = sentence.to_string();
        let mut last_input = current.clone();
        for model in stages {
            last_input = current.clone();
            current = cache.get(model, beam, &current)?;
        }
        Some((last_input, current))
    }

    /// Like [`Self::peek`] for a sentence with glossary terms: the finished text, if it is cached.
    fn peek_protected(&self, stages: &[ModelSpec], p: &Protected, beam: usize, inline_ok: bool) -> Option<(String, String)> {
        if p.terms.is_empty() {
            return self.peek(stages, &p.text, beam);
        }
        if inline_ok {
            // The inline attempt decides: accepted, or the placeholder form is the answer.
            let (last_input, text) = self.peek(stages, &p.inline, beam)?;
            if p.all_terms_in(&text) {
                return Some((last_input, exact_terms(&text, p)));
            }
        }
        self.peek(stages, &p.text, beam).map(|(last_input, text)| (last_input, restore(&text, p).text))
    }

    /// Translate sentences that may contain glossary terms. A sentence without terms goes through
    /// as it is. With terms, the target term is first written into the source sentence; if the
    /// translation keeps it, that is the result. Otherwise the term is swapped for a placeholder,
    /// translated, and the placeholder replaced by the target term afterwards.
    fn run_protected(&self, stages: &[ModelSpec], items: &[&Protected], beam: usize, inline_ok: bool) -> Result<Vec<(String, String)>, TranslateError> {
        let first: Vec<String> = items.iter().map(|p| if inline_ok && !p.terms.is_empty() { p.inline.clone() } else { p.text.clone() }).collect();
        let mut out = self.run_stages(stages, &first, beam)?;
        let mut retry = Vec::new();
        for (i, p) in items.iter().enumerate() {
            if p.terms.is_empty() {
                continue;
            }
            if inline_ok {
                if p.all_terms_in(&out[i].1) {
                    out[i].1 = exact_terms(&out[i].1, p);
                } else {
                    retry.push(i);
                }
            } else {
                out[i].1 = restore(&out[i].1, p).text;
            }
        }
        if !retry.is_empty() {
            let inputs: Vec<String> = retry.iter().map(|&i| items[i].text.clone()).collect();
            let second = self.run_stages(stages, &inputs, beam)?;
            for (&i, (last_input, text)) in retry.iter().zip(second) {
                out[i] = (last_input, restore(&text, items[i]).text);
            }
        }
        Ok(out)
    }

    /// Run sentences through every stage, translating only what is not cached. Returns, per
    /// sentence, what the last stage read and what it wrote.
    fn run_stages(&self, stages: &[ModelSpec], sentences: &[String], beam: usize) -> Result<Vec<(String, String)>, TranslateError> {
        let mut current = sentences.to_vec();
        let mut last_inputs = current.clone();
        for model in stages {
            last_inputs = current.clone();
            current = self.run_cached(model, &current, beam)?;
        }
        Ok(last_inputs.into_iter().zip(current).collect())
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

/// Where each translated sentence sits in the final text. Sentences are found in order, so equal
/// sentences cannot be confused; if one cannot be found (it should always be there) there are no
/// segments and the app simply offers no alternatives.
fn build_segments(text: &str, sources: &[String], inputs: &[String], outputs: &[Option<String>], pinned: &[bool]) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut cursor = 0;
    for i in 0..sources.len() {
        let Some(target) = outputs[i].as_deref() else { return Vec::new() };
        let Some(at) = text[cursor..].find(target) else { return Vec::new() };
        let (start_byte, end_byte) = (cursor + at, cursor + at + target.len());
        let utf16 = |upto: usize| text[..upto].encode_utf16().count();
        segments.push(Segment {
            source: sources[i].clone(),
            input: inputs[i].clone(),
            target: target.to_string(),
            start: utf16(start_byte),
            end: utf16(end_byte),
            pinned: pinned[i],
        });
        cursor = end_byte;
    }
    segments
}

/// Where each glossary term stands in the final text: found in order inside its sentence's translation.
/// Needs the segments (they give each sentence's position); a pinned sentence has none, it was chosen by hand.
fn term_hits(segments: &[Segment], protected: &[Protected], outputs: &[Option<String>]) -> Vec<TermHit> {
    let mut hits = Vec::new();
    for (i, seg) in segments.iter().enumerate() {
        if seg.pinned {
            continue;
        }
        let (Some(p), Some(out)) = (protected.get(i), outputs.get(i).and_then(|o| o.as_deref())) else { continue };
        let mut cursor = 0;
        for (k, (_, target)) in p.terms.iter().enumerate() {
            if target.is_empty() {
                continue;
            }
            let Some(at) = out[cursor..].find(target.as_str()) else { continue };
            let (from_byte, to_byte) = (cursor + at, cursor + at + target.len());
            let before = out[..from_byte].encode_utf16().count();
            let len = out[from_byte..to_byte].encode_utf16().count();
            hits.push(TermHit { start: seg.start + before, end: seg.start + before + len, source: p.sources.get(k).cloned().unwrap_or_default(), target: target.clone() });
            cursor = to_byte;
        }
    }
    hits
}

/// Glossary terms of a sentence that are in its (restored) translation: restoring replaced each
/// placeholder by its target term, so count the target terms that are present.
fn restore_count(protected: &Protected, restored_text: &str) -> usize {
    protected.terms.iter().filter(|(_, target)| !target.is_empty() && restored_text.contains(target.as_str())).count()
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
        assert_eq!(out, Translation::plain("Hello.", false));
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

    // --- glossary and pinned choices ---------------------------------------------------------------------------

    fn entry(source: &str, target: &str) -> crate::GlossaryEntry {
        crate::GlossaryEntry { id: 1, from: "en".into(), to: "de".into(), source: source.into(), target: target.into(), match_case: false, whole_word: true, enabled: true }
    }

    fn glossary(entries: Vec<crate::GlossaryEntry>) -> Glossary {
        Glossary { enabled: true, entries }
    }

    #[test]
    fn a_pinned_choice_is_dropped_when_the_glossary_starts_to_treat_the_sentence_differently() {
        let r = router(ALL);
        let first = r.translate("I'm astonished.", "en", "de", &no_cancel()).unwrap();
        let seg = &first.segments[0];
        r.pin("en", "de", &seg.source, &seg.input, "Ich bin \u{fc}berrascht.");
        assert_eq!(r.translate("I'm astonished.", "en", "de", &no_cancel()).unwrap().text, "Ich bin \u{fc}berrascht.", "the pick holds without a glossary");

        // A term for a word in this sentence is added: the pick is no longer valid, the glossary decides.
        r.set_glossary(glossary(vec![entry("astonished", "xsdcv")]));
        let out = r.translate("I'm astonished.", "en", "de", &no_cancel()).unwrap();
        assert!(out.text.contains("xsdcv"), "{}", out.text);
        assert!(!out.segments[0].pinned);
        assert_eq!(out.glossary_hits, 1);

        // Picking again under the new glossary holds, until the term changes or is switched off.
        r.pin("en", "de", "I'm astonished.", &out.segments[0].input, "Ich bin xsdcv hier.");
        assert_eq!(r.translate("I'm astonished.", "en", "de", &no_cancel()).unwrap().text, "Ich bin xsdcv hier.");
        r.set_glossary(glossary(vec![entry("astonished", "qwerty")]));
        assert!(r.translate("I'm astonished.", "en", "de", &no_cancel()).unwrap().text.contains("qwerty"), "an edited term wins over the old pick");
    }

    #[test]
    fn a_term_in_another_sentence_leaves_the_pick_alone_and_a_disabled_term_counts_as_a_change() {
        let r = router(ALL);
        let first = r.translate("Hello. I'm astonished.", "en", "de", &no_cancel()).unwrap();
        let seg = &first.segments[0];
        r.pin("en", "de", &seg.source, &seg.input, "Hallo du.");
        r.set_glossary(glossary(vec![entry("astonished", "xsdcv")]));
        let out = r.translate("Hello. I'm astonished.", "en", "de", &no_cancel()).unwrap();
        assert!(out.segments[0].pinned && out.text.starts_with("Hallo du."), "{}", out.text);
        assert!(out.text.contains("xsdcv"));

        // A pinned sentence with a term, then the term is disabled: the sentence is translated again.
        r.pin("en", "de", &out.segments[1].source, &out.segments[1].input, "Alt xsdcv.");
        let mut off = entry("astonished", "xsdcv");
        off.enabled = false;
        r.set_glossary(glossary(vec![off]));
        assert!(!r.translate("Hello. I'm astonished.", "en", "de", &no_cancel()).unwrap().text.contains("Alt xsdcv"));
    }

    /// Translates a made-up word away ("xsdcv" -> "foo") but copies the placeholder tokens through.
    struct DropsMadeUpWords;
    impl Runner for DropsMadeUpWords {
        fn run(&self, _m: &ModelSpec, sentences: &[String], _b: usize) -> Result<Vec<String>, TranslateError> {
            Ok(sentences.iter().map(|s| s.replace("xsdcv", "foo")).collect())
        }
    }

    #[test]
    fn a_made_up_target_survives_through_the_placeholder_when_the_model_changes_it() {
        let r = Router::new(FakeResolver(ALL.iter().copied().collect()), DropsMadeUpWords, "en");
        r.set_glossary(glossary(vec![entry("astonished", "xsdcv")]));
        let out = r.translate("I'm astonished.", "en", "de", &no_cancel()).unwrap();
        assert_eq!(out.text, "I'm xsdcv.", "the inline form lost the word, so the placeholder form put it in");
        assert_eq!(out.glossary_hits, 1);
        assert_eq!(out.glossary_terms.len(), 1);
        assert_eq!((out.glossary_terms[0].source.as_str(), out.glossary_terms[0].target.as_str()), ("astonished", "xsdcv"));
        assert_eq!(&out.text[out.glossary_terms[0].start..out.glossary_terms[0].end], "xsdcv");
    }

    #[test]
    fn the_cache_does_not_serve_a_translation_made_under_another_glossary() {
        let r = router(ALL);
        let plain = r.translate("The owl sleeps.", "en", "de", &no_cancel()).unwrap().text;
        r.set_glossary(glossary(vec![entry("owl", "Eule")]));
        let with = r.translate("The owl sleeps.", "en", "de", &no_cancel()).unwrap().text;
        assert_ne!(plain, with);
        assert!(with.contains("Eule"));
        r.set_glossary(Glossary::default());
        assert_eq!(r.translate("The owl sleeps.", "en", "de", &no_cancel()).unwrap().text, plain, "switching it off brings the plain translation back");
    }

    #[test]
    fn glossary_terms_are_located_in_the_final_text_across_sentences() {
        let r = router(ALL);
        r.set_glossary(glossary(vec![entry("owl", "Eule")]));
        let out = r.translate("Hello there. The owl sleeps.", "en", "de", &no_cancel()).unwrap();
        assert_eq!(out.glossary_terms.len(), 1);
        let t = &out.glossary_terms[0];
        assert_eq!(&out.text[t.start..t.end], "Eule");
        assert_eq!(t.source, "owl");
    }
}
