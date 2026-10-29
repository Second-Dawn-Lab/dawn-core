//! Model catalog, download manager and the shared model store
//! (`%AppData%/SecondDawn/models` on Windows).
//!
//! This crate is the only code in Second Dawn allowed to use the network (see [`download`]).

pub mod catalog;
pub mod download;
pub mod error;
pub mod store;

pub use catalog::{Catalog, Language, Model};
pub use error::{ModelError, Result};
pub use store::{sha256_file, InstallOptions, InstalledModel, LanguageSizes, ModelStore, Step, StepKind};
