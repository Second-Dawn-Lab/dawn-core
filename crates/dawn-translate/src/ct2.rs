//! CTranslate2 backend: runs Opus-MT (Marian) models from the shared model store.
//!
//! Models are loaded lazily on first use. A small LRU keeps at most `max_loaded` models in
//! memory, and a background sweeper unloads models that have been idle for a while.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use anyhow::Result;
use dawn_models::{Catalog, ModelStore};
use ct2rs::{Config, TranslationOptions, Translator};

use crate::lang_token::LangTokenizer;
use crate::router::{ModelSpec, Resolver, Router, Runner, TranslateError, TranslateOptions, Translation, Update};
use crate::TranslationEngine;

type LoadedModel = Translator<LangTokenizer>;

#[derive(Debug, Clone)]
pub struct Ct2Options {
    /// CPU threads per model (0 = pick a share of the cores).
    pub threads: usize,
    /// Models kept in memory at once.
    pub max_loaded: usize,
    /// Unload a model after this long without use.
    pub idle_unload: Duration,
}

impl Default for Ct2Options {
    fn default() -> Self {
        Self { threads: 0, max_loaded: 4, idle_unload: Duration::from_secs(5 * 60) }
    }
}

/// About two thirds of the logical cores, at least two: fast without freezing the machine.
/// (On a 6-core CPU 4 threads measured fastest; 6 was slower than 4.)
pub fn default_threads() -> usize {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    (cores * 2 / 3).max(2)
}

/// Finds installed models in the shared store, using the catalog for directions.
pub struct StoreResolver {
    pub store: ModelStore,
    pub catalog: Arc<Catalog>,
}

impl Resolver for StoreResolver {
    fn resolve(&self, from: &str, to: &str) -> Option<ModelSpec> {
        self.store.find_model(&self.catalog, from, to).map(|(model, dir)| ModelSpec { dir, prefix: model.target_token(to).map(str::to_string) })
    }
}

struct Entry {
    dir: PathBuf,
    model: Arc<LoadedModel>,
    last_used: Instant,
}

struct Cache {
    entries: Vec<Entry>,
    max_loaded: usize,
    idle_unload: Duration,
}

impl Cache {
    fn evict_idle(&mut self) {
        let idle = self.idle_unload;
        self.entries.retain(|e| e.last_used.elapsed() < idle);
    }
}

pub struct Ct2Runner {
    cache: Arc<Mutex<Cache>>,
    threads: AtomicUsize,
}

impl Ct2Runner {
    pub fn new(options: &Ct2Options) -> Self {
        let cache = Arc::new(Mutex::new(Cache { entries: Vec::new(), max_loaded: options.max_loaded.max(1), idle_unload: options.idle_unload }));
        spawn_sweeper(Arc::downgrade(&cache));
        let threads = if options.threads == 0 { default_threads() } else { options.threads };
        Self { cache, threads: AtomicUsize::new(threads) }
    }

    pub fn loaded_count(&self) -> usize {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).entries.len()
    }

    pub fn unload_all(&self) {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).entries.clear();
    }

    /// Threads per model (0 = automatic). Loaded models are dropped so they reload with it.
    pub fn set_threads(&self, threads: usize) {
        let threads = if threads == 0 { default_threads() } else { threads };
        if self.threads.swap(threads, Ordering::Relaxed) != threads {
            self.unload_all();
        }
    }

    pub fn threads(&self) -> usize {
        self.threads.load(Ordering::Relaxed)
    }

    fn get(&self, spec: &ModelSpec) -> Result<Arc<LoadedModel>, TranslateError> {
        {
            let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(entry) = cache.entries.iter_mut().find(|e| e.dir == spec.dir) {
                entry.last_used = Instant::now();
                return Ok(entry.model.clone());
            }
        }
        // Load outside the lock so other models stay usable meanwhile.
        let model = Arc::new(self.load(spec)?);
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.entries.retain(|e| e.dir != spec.dir);
        cache.entries.push(Entry { dir: spec.dir.clone(), model: model.clone(), last_used: Instant::now() });
        while cache.entries.len() > cache.max_loaded {
            let oldest = cache.entries.iter().enumerate().min_by_key(|(_, e)| e.last_used).map(|(i, _)| i).unwrap_or(0);
            cache.entries.remove(oldest);
        }
        Ok(model)
    }

    fn load(&self, spec: &ModelSpec) -> Result<LoadedModel, TranslateError> {
        let name = spec.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let fail = |reason: String| TranslateError::ModelLoad { model: name.clone(), reason };
        let tokenizer = LangTokenizer::open(&spec.dir).map_err(|e| fail(e.to_string()))?;
        let config = Config { num_threads_per_replica: self.threads(), ..Config::default() };
        Translator::with_tokenizer(&spec.dir, tokenizer, &config).map_err(|e| fail(e.to_string()))
    }
}

impl Runner for Ct2Runner {
    fn run(&self, spec: &ModelSpec, sentences: &[String], beam: usize) -> Result<Vec<String>, TranslateError> {
        let model = self.get(spec)?;
        let inputs: Vec<String> = match &spec.prefix {
            Some(prefix) => sentences.iter().map(|s| format!("{prefix} {s}")).collect(),
            None => sentences.to_vec(),
        };
        let options = TranslationOptions::<String, String> { beam_size: beam, max_decoding_length: 512, ..Default::default() };
        let out = model.translate_batch(&inputs, &options, None).map_err(|e| TranslateError::Engine(e.to_string()))?;
        let strip = spec.prefix.is_some();
        Ok(out
            .into_iter()
            .map(|(text, _)| if strip { strip_language_tokens(&text) } else { text.trim().to_string() })
            .collect())
    }
}

/// Multi-target models sometimes copy their language token into the output (seen with
/// `opus-mt-en-ar`: `... >>ara>> ...`). Remove anything shaped like `>>name<<` or `>>name>>`.
fn strip_language_tokens(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(">>") {
        let after = &rest[start + 2..];
        let name_len = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(after.len());
        let tail = &after[name_len..];
        if name_len > 0 && (tail.starts_with("<<") || tail.starts_with(">>")) {
            out.push_str(&rest[..start]);
            rest = &tail[2..];
        } else {
            out.push_str(&rest[..start + 2]);
            rest = after;
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::strip_language_tokens;

    #[test]
    fn removes_leaked_language_tokens() {
        assert_eq!(strip_language_tokens("مرحبا >>ara>> أتمنى"), "مرحبا أتمنى");
        assert_eq!(strip_language_tokens(">>cmn_Hans<< 你好"), "你好");
        assert_eq!(strip_language_tokens("a >> b << c"), "a >> b << c");
        assert_eq!(strip_language_tokens("  plain text "), "plain text");
    }
}

fn spawn_sweeper(cache: Weak<Mutex<Cache>>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(30));
        match cache.upgrade() {
            Some(cache) => cache.lock().unwrap_or_else(|e| e.into_inner()).evict_idle(),
            None => break,
        }
    });
}

/// The real engine: router + CTranslate2 runner over the shared model store.
pub struct Ct2Engine {
    router: Router<StoreResolver, Ct2Runner>,
    store: ModelStore,
    catalog: Arc<Catalog>,
}

impl Ct2Engine {
    pub fn new(store: ModelStore, catalog: Arc<Catalog>, options: &Ct2Options) -> Self {
        let resolver = StoreResolver { store: store.clone(), catalog: catalog.clone() };
        let router = Router::new(resolver, Ct2Runner::new(options), catalog.hub.clone());
        Self { router, store, catalog }
    }

    /// Translate and report whether English was used as a pivot. Set `cancel` to abandon a
    /// request that a newer one has replaced.
    pub fn translate_detailed(&self, text: &str, from: &str, to: &str, cancel: &AtomicBool) -> Result<Translation, TranslateError> {
        self.router.translate(text, from, to, cancel)
    }

    /// Translate in small batches, reporting each finished batch (see [`TranslateOptions`]).
    pub fn translate_streaming(
        &self,
        text: &str,
        from: &str,
        to: &str,
        cancel: &AtomicBool,
        options: &TranslateOptions,
        on_update: &mut dyn FnMut(Update),
    ) -> Result<Translation, TranslateError> {
        self.router.translate_streaming(text, from, to, cancel, options, on_update)
    }

    /// Threads per model (0 = automatic). Takes effect for the next translation.
    pub fn set_threads(&self, threads: usize) {
        self.router_runner().set_threads(threads);
    }

    pub fn threads(&self) -> usize {
        self.router_runner().threads()
    }

    /// Forget the per-sentence translation cache (benchmarks, or after changing models).
    pub fn clear_cache(&self) {
        self.router.clear_cache();
    }

    pub fn loaded_models(&self) -> usize {
        self.router_runner().loaded_count()
    }

    pub fn unload_all(&self) {
        self.router_runner().unload_all();
    }

    fn router_runner(&self) -> &Ct2Runner {
        self.router.runner()
    }

    /// English plus every installed language.
    pub fn installed_languages(&self) -> Vec<String> {
        let mut langs = vec![self.catalog.hub.clone()];
        langs.extend(self.catalog.languages.iter().filter(|l| self.store.is_language_installed(&self.catalog, &l.code)).map(|l| l.code.clone()));
        langs
    }
}

impl TranslationEngine for Ct2Engine {
    fn translate(&self, text: &str, from: &str, to: &str) -> Result<String> {
        Ok(self.translate_detailed(text, from, to, &AtomicBool::new(false))?.text)
    }

    fn supported_pairs(&self) -> Vec<(String, String)> {
        let langs = self.installed_languages();
        langs.iter().flat_map(|a| langs.iter().filter(move |b| *b != a).map(move |b| (a.clone(), b.clone()))).collect()
    }
}
