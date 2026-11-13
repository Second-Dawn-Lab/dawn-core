//! The two backends behind one trait, which one is used, and when the model is loaded and unloaded.
//!
//! * `BuiltIn`: a `llama-server` started on demand from the downloaded runtime pack, serving one downloaded GGUF.
//! * `Ollama`: an Ollama already running on this computer; the user picks one of its installed models. Nothing is
//!   downloaded or installed for it.
//!
//! Nothing is loaded at app start: the first request loads the model, [`Llm::unload_if_idle`] frees it again.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::client::Client;
use crate::endpoint::{Endpoint, OLLAMA_DEFAULT};
use crate::error::{LlmError, Result};
use crate::memory::{check_memory, system_memory, MemoryVerdict};
use crate::prompts::Prompt;
use crate::sidecar::{Sidecar, SidecarSpec};

/// What the user chose in Settings, Quality mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Choice {
    #[default]
    Off,
    /// The downloaded runtime and model.
    Builtin,
    /// A local Ollama.
    Ollama,
}

/// A language model that answers one prompt.
pub trait Backend: Send {
    fn name(&self) -> &'static str;
    fn complete(&self, prompt: &Prompt, cancel: &AtomicBool) -> Result<String>;
}

struct BuiltIn {
    // Held for its Drop: the server stops with it.
    _sidecar: Sidecar,
    client: Client,
}

impl Backend for BuiltIn {
    fn name(&self) -> &'static str {
        "builtin"
    }
    fn complete(&self, prompt: &Prompt, cancel: &AtomicBool) -> Result<String> {
        self.client.complete(prompt, cancel)
    }
}

struct OllamaBackend {
    client: Client,
}

impl Backend for OllamaBackend {
    fn name(&self) -> &'static str {
        "ollama"
    }
    fn complete(&self, prompt: &Prompt, cancel: &AtomicBool) -> Result<String> {
        self.client.complete(prompt, cancel)
    }
}

/// Everything needed to pick and start a backend. The app builds it from the settings and the installed packs.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub choice: Choice,
    /// The runtime and model files, when both are installed.
    pub builtin: Option<SidecarSpec>,
    /// Memory the built-in model needs (the pack's `ramBytes`).
    pub builtin_ram: u64,
    pub ollama_endpoint: String,
    pub ollama_model: String,
}

impl Config {
    pub fn off() -> Self {
        Self { choice: Choice::Off, builtin: None, builtin_ram: 0, ollama_endpoint: OLLAMA_DEFAULT.into(), ollama_model: String::new() }
    }
}

/// What can be used right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Availability {
    pub builtin_installed: bool,
    /// The models of a running Ollama; `None` when none runs.
    pub ollama_models: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selected {
    Off,
    BuiltIn,
    Ollama { model: String },
    Unavailable(String),
}

/// Which backend a choice leads to, and why it does not when it does not.
pub fn select_backend(choice: Choice, ollama_model: &str, avail: &Availability) -> Selected {
    match choice {
        Choice::Off => Selected::Off,
        Choice::Builtin if avail.builtin_installed => Selected::BuiltIn,
        Choice::Builtin => Selected::Unavailable("download the Quality mode runtime and a model first".into()),
        Choice::Ollama => match &avail.ollama_models {
            None => Selected::Unavailable("Ollama is not running on this computer".into()),
            Some(models) if models.is_empty() => Selected::Unavailable("Ollama has no models installed".into()),
            Some(models) if ollama_model.is_empty() => {
                let _ = models;
                Selected::Unavailable("choose one of your Ollama models".into())
            }
            Some(models) if models.iter().any(|m| m == ollama_model) => Selected::Ollama { model: ollama_model.to_string() },
            Some(_) => Selected::Unavailable(format!("the Ollama model \"{ollama_model}\" is not installed")),
        },
    }
}

#[derive(Default)]
struct State {
    /// The loaded backend and what it was loaded for.
    loaded: Option<(String, Box<dyn Backend>)>,
    last_used: Option<Instant>,
}

type Starter = Box<dyn Fn(&Config) -> Result<Box<dyn Backend>> + Send + Sync>;

pub struct Llm {
    state: Mutex<State>,
    starter: Starter,
    /// A backend is being started (the model is loading): the settings page and the result pane say "Loading model...".
    loading: AtomicBool,
    /// Told when the model starts loading, is loaded, or is freed (the app refreshes its status line).
    observer: Mutex<Option<std::sync::Arc<dyn Fn() + Send + Sync>>>,
}

impl Default for Llm {
    fn default() -> Self {
        Self::with_starter(Box::new(Llm::start))
    }
}

/// How long the built-in model may take to load.
const LOAD_TIMEOUT: Duration = Duration::from_secs(180);

impl Llm {
    pub fn new() -> Self {
        Self::default()
    }

    /// A model host with its own way to start a backend (tests use a fake one that counts loads).
    pub fn with_starter(starter: Starter) -> Self {
        Self { state: Mutex::new(State::default()), starter, loading: AtomicBool::new(false), observer: Mutex::new(None) }
    }

    /// Be told (from any thread) when the loading state changes.
    pub fn set_observer(&self, observer: std::sync::Arc<dyn Fn() + Send + Sync>) {
        *self.observer.lock().unwrap_or_else(|e| e.into_inner()) = Some(observer);
    }

    fn changed(&self) {
        let observer = self.observer.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(observer) = observer {
            observer();
        }
    }

    /// The model is being loaded right now.
    pub fn is_loading(&self) -> bool {
        self.loading.load(Ordering::Relaxed)
    }

    fn key(cfg: &Config) -> String {
        match cfg.choice {
            Choice::Off => "off".into(),
            Choice::Builtin => format!("builtin:{:?}", cfg.builtin.as_ref().map(|s| (&s.model, s.threads, s.context))),
            Choice::Ollama => format!("ollama:{}:{}", cfg.ollama_endpoint, cfg.ollama_model),
        }
    }

    fn start(cfg: &Config) -> Result<Box<dyn Backend>> {
        match cfg.choice {
            Choice::Off => Err(LlmError::Unavailable("Quality mode is off".into())),
            Choice::Builtin => {
                let spec = cfg.builtin.as_ref().ok_or_else(|| LlmError::Unavailable("the runtime or the model is not installed".into()))?;
                if let Some(info) = system_memory() {
                    if let MemoryVerdict::TooLittleFree { need, free } = check_memory(cfg.builtin_ram, info) {
                        return Err(LlmError::LowMemory { need, free });
                    }
                }
                let sidecar = Sidecar::start(spec, LOAD_TIMEOUT)?;
                let client = sidecar.client.clone();
                Ok(Box::new(BuiltIn { _sidecar: sidecar, client }))
            }
            Choice::Ollama => {
                if cfg.ollama_model.is_empty() {
                    return Err(LlmError::Unavailable("choose one of your Ollama models".into()));
                }
                let endpoint = Endpoint::parse(&cfg.ollama_endpoint)?;
                Ok(Box::new(OllamaBackend { client: Client::new(endpoint, cfg.ollama_model.clone()) }))
            }
        }
    }

    /// Answer one prompt, loading the model first if it is not loaded. One request at a time; a request that is
    /// waiting for the previous one to stop should set that one's `cancel` flag first.
    pub fn complete(&self, cfg: &Config, prompt: &Prompt, cancel: &AtomicBool) -> Result<String> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let key = Self::key(cfg);
        if state.loaded.as_ref().is_some_and(|(k, _)| *k != key) {
            state.loaded = None; // another model or backend: the old one is stopped
        }
        // Loaded again on demand: after the idle unload (or a crashed runtime) the next request starts it afresh.
        if state.loaded.is_none() {
            self.loading.store(true, Ordering::Relaxed);
            self.changed();
            let started = (self.starter)(cfg);
            self.loading.store(false, Ordering::Relaxed);
            match started {
                Ok(backend) => {
                    state.loaded = Some((key, backend));
                    self.changed();
                }
                Err(e) => {
                    self.changed();
                    return Err(e);
                }
            }
        }
        state.last_used = Some(Instant::now());
        let result = state.loaded.as_ref().map(|(_, b)| b.complete(prompt, cancel)).unwrap_or(Err(LlmError::Cancelled));
        state.last_used = Some(Instant::now());
        // A server that stopped answering is dropped so the next request starts a fresh one.
        if matches!(result, Err(LlmError::Server(_))) {
            state.loaded = None;
        }
        result
    }

    pub fn is_loaded(&self) -> bool {
        self.state.try_lock().map(|s| s.loaded.is_some()).unwrap_or(true)
    }

    /// Free the model now (the app quits, Quality mode is switched off, or the user clicked "Unload").
    pub fn unload(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.loaded = None;
        state.last_used = None;
        drop(state);
        self.changed();
    }

    /// Free the model when it has not been used for `idle`. Does nothing while a request runs. Returns true if it unloaded.
    pub fn unload_if_idle(&self, idle: Duration, now: Instant) -> bool {
        let Ok(mut state) = self.state.try_lock() else { return false };
        let expired = state.loaded.is_some() && state.last_used.is_some_and(|t| now.saturating_duration_since(t) >= idle);
        if expired {
            state.loaded = None;
            state.last_used = None;
            drop(state);
            self.changed();
        }
        expired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn avail(builtin: bool, ollama: Option<&[&str]>) -> Availability {
        Availability { builtin_installed: builtin, ollama_models: ollama.map(|m| m.iter().map(|s| s.to_string()).collect()) }
    }

    #[test]
    fn off_is_off_whatever_is_installed() {
        assert_eq!(select_backend(Choice::Off, "", &avail(true, Some(&["a"]))), Selected::Off);
    }

    #[test]
    fn the_built_in_backend_needs_its_packs() {
        assert_eq!(select_backend(Choice::Builtin, "", &avail(true, None)), Selected::BuiltIn);
        assert!(matches!(select_backend(Choice::Builtin, "", &avail(false, Some(&["a"]))), Selected::Unavailable(_)));
    }

    #[test]
    fn ollama_needs_a_running_server_and_an_installed_model() {
        assert_eq!(select_backend(Choice::Ollama, "qwen3:4b", &avail(false, Some(&["qwen3:4b", "x"]))), Selected::Ollama { model: "qwen3:4b".into() });
        for (model, a) in [("qwen3:4b", avail(true, None)), ("qwen3:4b", avail(true, Some(&[]))), ("", avail(true, Some(&["x"]))), ("gone", avail(true, Some(&["x"])))] {
            assert!(matches!(select_backend(Choice::Ollama, model, &a), Selected::Unavailable(_)), "{model:?}");
        }
    }

    #[test]
    fn choosing_ollama_does_not_fall_back_to_the_built_in_model() {
        assert!(matches!(select_backend(Choice::Ollama, "x", &avail(true, None)), Selected::Unavailable(_)));
    }

    #[test]
    fn nothing_is_loaded_until_a_request_and_off_never_loads() {
        let llm = Llm::new();
        assert!(!llm.is_loaded());
        let prompt = Prompt { user: "x".into(), max_tokens: 8 };
        assert!(matches!(llm.complete(&Config::off(), &prompt, &AtomicBool::new(false)), Err(LlmError::Unavailable(_))));
        assert!(!llm.is_loaded());
    }

    #[test]
    fn a_remote_ollama_address_is_refused() {
        let cfg = Config { choice: Choice::Ollama, ollama_endpoint: "192.168.1.9:11434".into(), ollama_model: "m".into(), ..Config::off() };
        let llm = Llm::new();
        let prompt = Prompt { user: "x".into(), max_tokens: 8 };
        assert!(matches!(llm.complete(&cfg, &prompt, &AtomicBool::new(false)), Err(LlmError::NotLocal(_))));
    }

    struct Fake;
    impl Backend for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn complete(&self, _: &Prompt, _: &AtomicBool) -> Result<String> {
            Ok("ok".into())
        }
    }

    #[test]
    fn an_idle_model_is_unloaded_and_a_busy_or_fresh_one_is_not() {
        let llm = Llm::new();
        let t0 = Instant::now();
        {
            let mut s = llm.state.lock().unwrap();
            s.loaded = Some(("k".into(), Box::new(Fake)));
            s.last_used = Some(t0);
        }
        assert!(!llm.unload_if_idle(Duration::from_secs(300), t0 + Duration::from_secs(120)), "used two minutes ago");
        assert!(llm.is_loaded());
        assert!(llm.unload_if_idle(Duration::from_secs(300), t0 + Duration::from_secs(301)));
        assert!(!llm.is_loaded());
        assert!(!llm.unload_if_idle(Duration::from_secs(300), t0 + Duration::from_secs(900)), "nothing left to unload");
    }

    /// A backend that counts how often it was started.
    fn counting(starts: &std::sync::Arc<std::sync::atomic::AtomicUsize>) -> Llm {
        let starts = starts.clone();
        Llm::with_starter(Box::new(move |_| {
            starts.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Fake))
        }))
    }

    fn builtin_cfg() -> Config {
        Config { choice: Choice::Builtin, builtin: Some(SidecarSpec { exe: "e".into(), model: "m".into(), threads: 0, context: 1 }), ..Config::off() }
    }

    #[test]
    fn the_next_request_loads_the_model_again_after_the_idle_unload() {
        let starts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let llm = counting(&starts);
        let (cfg, cancel) = (builtin_cfg(), AtomicBool::new(false));
        let prompt = Prompt { user: "x".into(), max_tokens: 8 };
        assert_eq!(llm.complete(&cfg, &prompt, &cancel).unwrap(), "ok");
        assert_eq!(llm.complete(&cfg, &prompt, &cancel).unwrap(), "ok");
        assert_eq!(starts.load(Ordering::SeqCst), 1, "the loaded model is reused");
        assert!(llm.unload_if_idle(Duration::ZERO, Instant::now() + Duration::from_secs(1)));
        assert!(!llm.is_loaded());
        assert_eq!(llm.complete(&cfg, &prompt, &cancel).unwrap(), "ok", "unload, then the next request reloads");
        assert!(llm.is_loaded());
        assert_eq!(starts.load(Ordering::SeqCst), 2);
        assert!(!llm.is_loading());
    }

    #[test]
    fn a_failed_load_is_an_error_and_the_next_request_tries_again() {
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = attempts.clone();
        let llm = Llm::with_starter(Box::new(move |_| {
            if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(LlmError::LowMemory { need: 3, free: 1 })
            } else {
                Ok(Box::new(Fake))
            }
        }));
        let (cfg, cancel) = (builtin_cfg(), AtomicBool::new(false));
        let prompt = Prompt { user: "x".into(), max_tokens: 8 };
        assert!(matches!(llm.complete(&cfg, &prompt, &cancel), Err(LlmError::LowMemory { .. })));
        assert!(!llm.is_loaded() && !llm.is_loading());
        assert_eq!(llm.complete(&cfg, &prompt, &cancel).unwrap(), "ok");
    }

    #[test]
    fn a_runtime_that_stopped_answering_is_dropped_and_restarted() {
        struct Dead;
        impl Backend for Dead {
            fn name(&self) -> &'static str {
                "dead"
            }
            fn complete(&self, _: &Prompt, _: &AtomicBool) -> Result<String> {
                Err(LlmError::Server("gone".into()))
            }
        }
        let starts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = starts.clone();
        let llm = Llm::with_starter(Box::new(move |_| {
            if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(Box::new(Dead))
            } else {
                Ok(Box::new(Fake))
            }
        }));
        let (cfg, cancel) = (builtin_cfg(), AtomicBool::new(false));
        let prompt = Prompt { user: "x".into(), max_tokens: 8 };
        assert!(matches!(llm.complete(&cfg, &prompt, &cancel), Err(LlmError::Server(_))));
        assert_eq!(llm.complete(&cfg, &prompt, &cancel).unwrap(), "ok");
        assert_eq!(starts.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_model_in_use_is_not_unloaded() {
        let llm = Llm::new();
        {
            let mut s = llm.state.lock().unwrap();
            s.loaded = Some(("k".into(), Box::new(Fake)));
            s.last_used = Some(Instant::now());
        }
        let guard = llm.state.lock().unwrap();
        assert!(!llm.unload_if_idle(Duration::ZERO, Instant::now() + Duration::from_secs(10)));
        drop(guard);
    }
}
