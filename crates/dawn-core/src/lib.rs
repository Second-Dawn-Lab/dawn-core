//! Shared paths, configuration and logging for all Second Dawn apps.

use std::path::{Path, PathBuf};
use std::{env, fs, io};

use serde::{de::DeserializeOwned, Serialize};

/// Root data folder shared by all Second Dawn apps (`%AppData%/SecondDawn` on Windows).
pub fn data_dir() -> PathBuf {
    let base = env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| env::var_os("XDG_DATA_HOME").map(PathBuf::from))
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("SecondDawn")
}

/// The shared model store used by every app.
///
/// Development builds only: the environment variable `DAWN_MODELS_DIR` points the store at another
/// folder (for example an empty temporary one, to test pack downloads without touching the installed
/// packs). Release builds ignore it.
pub fn models_dir() -> PathBuf {
    let override_dir = if cfg!(debug_assertions) { env::var_os("DAWN_MODELS_DIR") } else { None };
    models_dir_with(override_dir)
}

fn models_dir_with(override_dir: Option<std::ffi::OsString>) -> PathBuf {
    match override_dir.filter(|d| !d.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => data_dir().join("models"),
    }
}

/// Per-app data folder, e.g. `%AppData%/SecondDawn/owll`.
pub fn app_dir(app: &str) -> PathBuf {
    data_dir().join(app)
}

/// Read a JSON file. A missing or unreadable file yields `T::default()`.
pub fn read_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Write a JSON file atomically (temp file + rename), creating parent folders.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    fs::rename(tmp, path)
}

// TODO: logging setup shared by all apps.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_models_dir_override_replaces_the_shared_store() {
        assert_eq!(models_dir_with(Some("D:/tmp/packs".into())), PathBuf::from("D:/tmp/packs"));
        assert_eq!(models_dir_with(Some("".into())), data_dir().join("models"), "an empty value is ignored");
        assert_eq!(models_dir_with(None), data_dir().join("models"));
    }
}
