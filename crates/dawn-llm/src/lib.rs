//! Quality mode: an optional local language model that polishes the fast translation, applies a tone, inflects glossary
//! terms, suggests word alternatives and rewrites text (the Write tab).
//!
//! Two backends sit behind one trait ([`Backend`]): a `llama-server` (llama.cpp) started from a downloaded runtime pack,
//! and a local Ollama. **Only this computer is ever contacted** ([`Endpoint`] refuses every other address); nothing is
//! loaded at app start and the model is freed after an idle time. The prompts are fixed ([`prompts`]) and every answer is
//! checked ([`validate`]); when a check fails, the caller keeps the fast translation.
//!
//! See `docs/quality-mode.md`.

pub mod client;
pub mod diff;
pub mod endpoint;
pub mod error;
pub mod memory;
pub mod prompts;
pub mod service;
pub mod sidecar;
pub mod tasks;
pub mod tone;
pub mod tone_check;
pub mod validate;

pub use client::{ollama_models, OllamaModel};
pub use endpoint::{Endpoint, OLLAMA_DEFAULT};
pub use error::{LlmError, Result};
pub use memory::{check_memory, system_memory, MemoryInfo, MemoryVerdict};
pub use prompts::{Prompt, Term, WriteKind};
pub use service::{select_backend, Availability, Backend, Choice, Config, Llm, Selected};
pub use sidecar::SidecarSpec;
pub use tone::{tone_available, tone_instruction, Tone};
