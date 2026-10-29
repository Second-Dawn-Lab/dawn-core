//! The shared model store (`%AppData%/SecondDawn/models`) and its `installed.json`.
//!
//! ```text
//! models/
//!   installed.json            list of installed models, shared by all Second Dawn apps
//!   <model-id>/               extracted model (model.bin, source.spm, ...)
//!   .downloads/<id>.zip.part  partial downloads (kept so they can resume)
//! ```
//!
//! A model can serve several languages. It is downloaded once, and removed only when no
//! installed language needs it any more. `installed.json` lists both the installed models and
//! the installed languages.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::catalog::{Catalog, Model};
use crate::download::download;
use crate::error::{ModelError, Result};

/// Serializes read-modify-write cycles of `installed.json` inside this process.
static INSTALLED_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModel {
    pub id: String,
    pub size_bytes: u64,
    pub sha256: String,
    /// Unix time in seconds.
    pub installed_at: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct InstalledFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    models: Vec<InstalledModel>,
    /// Active languages: the ones shown in the pickers. Several languages can share the same
    /// models, so having a language's models on disk does not make it active (it may only be
    /// there for another one); activating it is instant.
    #[serde(default)]
    languages: Vec<String>,
    /// Models for which the "activate the other languages this model supports?" question was
    /// asked already. Kept even if the files are deleted and downloaded again.
    #[serde(default)]
    offered_models: Vec<String>,
}

/// Steps of installing a language, for progress reporting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    /// Bytes downloaded so far across the language's missing models, and the expected total.
    Downloading { done: u64, total: u64 },
    Verifying,
    Extracting,
}

/// What a language costs in disk and download, given what is installed already.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LanguageSizes {
    /// Still to download (models that are not installed yet). 0 when everything is shared.
    pub download: u64,
    /// All of its models, installed or not.
    pub total: u64,
    /// Freed by removing it: installed models no other installed language needs.
    pub freeable: u64,
}

pub struct InstallOptions<'a> {
    pub proxy: Option<&'a str>,
    pub cancel: &'a AtomicBool,
}

#[derive(Debug, Clone)]
pub struct ModelStore {
    root: PathBuf,
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

impl ModelStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The store every Second Dawn app shares.
    pub fn open_default() -> Self {
        Self::new(dawn_core::models_dir())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn model_dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    fn installed_path(&self) -> PathBuf {
        self.root.join("installed.json")
    }

    fn read_installed(&self) -> InstalledFile {
        dawn_core::read_json(&self.installed_path())
    }

    fn write_installed(&self, file: &InstalledFile) -> Result<()> {
        dawn_core::write_json(&self.installed_path(), file)?;
        Ok(())
    }

    pub fn installed_models(&self) -> Vec<InstalledModel> {
        self.read_installed().models
    }

    /// Installed means: listed in `installed.json` and the folder still exists.
    pub fn is_installed(&self, id: &str) -> bool {
        self.model_dir(id).join("model.bin").exists() && self.read_installed().models.iter().any(|m| m.id == id)
    }

    /// The hub is built in; any other language must have been installed and have all its models.
    pub fn is_language_installed(&self, catalog: &Catalog, code: &str) -> bool {
        if code == catalog.hub {
            return true;
        }
        self.read_installed().languages.iter().any(|l| l == code)
            && catalog
                .language(code)
                .is_some_and(|l| !l.models.is_empty() && l.models.iter().all(|id| catalog.model(id).is_some() && self.is_installed(id)))
    }

    /// An installed model that can translate `from -> to` (the most specialized one), with its folder.
    pub fn find_model(&self, catalog: &Catalog, from: &str, to: &str) -> Option<(Model, PathBuf)> {
        catalog.models_serving(from, to).into_iter().find(|m| self.is_installed(&m.id)).map(|m| (m.clone(), self.model_dir(&m.id)))
    }

    /// Models of a language that still have to be downloaded.
    pub fn missing_models<'a>(&self, catalog: &'a Catalog, code: &str) -> Vec<&'a Model> {
        catalog.language_models(code).into_iter().filter(|m| !self.is_installed(&m.id)).collect()
    }

    /// Ids of models that installed languages other than `code` still need.
    fn needed_by_others(&self, catalog: &Catalog, code: &str) -> Vec<String> {
        catalog
            .languages
            .iter()
            .filter(|other| other.code != code && self.is_language_installed(catalog, &other.code))
            .flat_map(|other| other.models.iter().cloned())
            .collect()
    }

    fn mark_language(&self, code: &str, installed: bool) -> Result<()> {
        let _guard = INSTALLED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut file = self.read_installed();
        file.languages.retain(|l| l != code);
        if installed {
            file.languages.push(code.to_string());
        }
        self.write_installed(&file)
    }

    pub fn language_sizes(&self, catalog: &Catalog, code: &str) -> LanguageSizes {
        let models = catalog.language_models(code);
        let needed = self.needed_by_others(catalog, code);
        LanguageSizes {
            download: models.iter().filter(|m| !self.is_installed(&m.id)).map(|m| m.size_bytes).sum(),
            total: models.iter().map(|m| m.size_bytes).sum(),
            freeable: models.iter().filter(|m| self.is_installed(&m.id) && !needed.contains(&m.id)).map(|m| m.size_bytes).sum(),
        }
    }

    /// Download, verify, extract and register one model.
    ///
    /// `on_step` gets the per-model download progress as `(done, total)` bytes.
    pub fn install_model(&self, model: &Model, opts: &InstallOptions, on_step: &mut dyn FnMut(StepKind)) -> Result<()> {
        if model.url.trim().is_empty() || model.url == "TBD" || model.sha256.is_empty() || model.sha256 == "TBD" {
            return Err(ModelError::NotPublished(model.id.clone()));
        }
        let part = self.root.join(".downloads").join(format!("{}.zip.part", model.id));
        download(&model.url, &part, opts.proxy, opts.cancel, &mut |done, total| on_step(StepKind::Downloading(done, total)))?;

        on_step(StepKind::Verifying);
        let actual = sha256_file(&part)?;
        if !actual.eq_ignore_ascii_case(&model.sha256) {
            let _ = fs::remove_file(&part);
            return Err(ModelError::Checksum { expected: model.sha256.clone(), actual });
        }

        on_step(StepKind::Extracting);
        let tmp = self.root.join(format!(".tmp-{}", model.id));
        if tmp.exists() {
            fs::remove_dir_all(&tmp)?;
        }
        if let Err(err) = extract_zip(&part, &tmp) {
            let _ = fs::remove_dir_all(&tmp);
            return Err(err);
        }
        let target = self.model_dir(&model.id);
        if target.exists() {
            fs::remove_dir_all(&target)?;
        }
        fs::rename(&tmp, &target)?;

        let _guard = INSTALLED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut file = self.read_installed();
        file.version = 2;
        file.models.retain(|m| m.id != model.id);
        file.models.push(InstalledModel {
            id: model.id.clone(),
            size_bytes: fs::metadata(&part).map(|m| m.len()).unwrap_or(model.size_bytes),
            sha256: actual,
            installed_at: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        });
        self.write_installed(&file)?;
        let _ = fs::remove_file(&part);
        Ok(())
    }

    /// Install every missing model of a language. Progress is aggregated over what has to be
    /// downloaded; models another language already installed cost nothing.
    pub fn install_language(&self, catalog: &Catalog, code: &str, opts: &InstallOptions, on_step: &mut dyn FnMut(Step)) -> Result<()> {
        if catalog.language(code).is_none() {
            return Err(ModelError::UnknownLanguage(code.to_string()));
        }
        let missing = self.missing_models(catalog, code);
        let total: u64 = missing.iter().map(|m| m.size_bytes).sum();
        let mut finished: u64 = 0;
        for model in missing {
            let base = finished;
            self.install_model(model, opts, &mut |kind| {
                on_step(match kind {
                    StepKind::Downloading(done, _) => Step::Downloading { done: base + done, total },
                    StepKind::Verifying => Step::Verifying,
                    StepKind::Extracting => Step::Extracting,
                })
            })?;
            finished += model.size_bytes;
        }
        on_step(Step::Downloading { done: total, total });
        self.mark_language(code, true)
    }

    /// Remove a language's models, except those another installed language still needs.
    /// Returns the ids that were removed.
    pub fn uninstall_language(&self, catalog: &Catalog, code: &str) -> Result<Vec<String>> {
        let language = catalog.language(code).ok_or_else(|| ModelError::UnknownLanguage(code.to_string()))?;
        self.mark_language(code, false)?;
        let needed = self.needed_by_others(catalog, code);

        let mut removed = Vec::new();
        let _guard = INSTALLED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut file = self.read_installed();
        for id in &language.models {
            if needed.contains(id) {
                continue;
            }
            let dir = self.model_dir(id);
            if dir.exists() {
                fs::remove_dir_all(&dir)?;
            }
            let _ = fs::remove_file(self.root.join(".downloads").join(format!("{id}.zip.part")));
            file.models.retain(|m| &m.id != id);
            removed.push(id.clone());
        }
        self.write_installed(&file)?;
        Ok(removed)
    }
}

impl ModelStore {
    /// All of a language's models are on disk (the hub always is), whether or not it is active.
    pub fn is_language_available(&self, catalog: &Catalog, code: &str) -> bool {
        code == catalog.hub
            || catalog
                .language(code)
                .is_some_and(|l| !l.models.is_empty() && l.models.iter().all(|id| catalog.model(id).is_some() && self.is_installed(id)))
    }

    /// Codes of the active languages (not including the built-in hub).
    pub fn active_languages(&self) -> Vec<String> {
        self.read_installed().languages
    }

    /// Show a language whose models are already on disk in the pickers. Instant.
    pub fn activate_language(&self, catalog: &Catalog, code: &str) -> Result<()> {
        if catalog.language(code).is_none() {
            return Err(ModelError::UnknownLanguage(code.to_string()));
        }
        if !self.is_language_available(catalog, code) {
            return Err(ModelError::NotAvailable(code.to_string()));
        }
        self.mark_language(code, true)
    }

    /// Hide a language from the pickers. The files stay. Returns the ids of its models that no
    /// other active language needs any more (the caller can offer to delete them).
    pub fn deactivate_language(&self, catalog: &Catalog, code: &str) -> Result<Vec<String>> {
        let language = catalog.language(code).ok_or_else(|| ModelError::UnknownLanguage(code.to_string()))?;
        self.mark_language(code, false)?;
        let needed = self.needed_by_others(catalog, code);
        Ok(language.models.iter().filter(|id| self.is_installed(id) && !needed.contains(id)).cloned().collect())
    }

    /// Installed models that no active language needs.
    pub fn orphaned_models(&self, catalog: &Catalog) -> Vec<InstalledModel> {
        let needed: Vec<String> = catalog
            .languages
            .iter()
            .filter(|l| self.is_language_installed(catalog, &l.code))
            .flat_map(|l| l.models.iter().cloned())
            .collect();
        self.read_installed().models.into_iter().filter(|m| !needed.contains(&m.id)).collect()
    }

    /// Delete model files that no active language needs. Models that are needed again by now are
    /// left alone. Returns the bytes freed.
    pub fn delete_orphaned(&self, catalog: &Catalog, ids: &[String]) -> Result<u64> {
        let orphans: Vec<InstalledModel> = self.orphaned_models(catalog).into_iter().filter(|m| ids.contains(&m.id)).collect();
        let freed = orphans.iter().map(|m| catalog.model(&m.id).map_or(m.size_bytes, |c| c.size_bytes)).sum();
        self.remove_model_files(&orphans.iter().map(|m| m.id.clone()).collect::<Vec<_>>())?;
        Ok(freed)
    }

    /// Delete every model file of a language, even ones other languages share (used to repair).
    pub fn delete_language_files(&self, catalog: &Catalog, code: &str) -> Result<()> {
        let language = catalog.language(code).ok_or_else(|| ModelError::UnknownLanguage(code.to_string()))?;
        self.remove_model_files(&language.models)
    }

    fn remove_model_files(&self, ids: &[String]) -> Result<()> {
        let _guard = INSTALLED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut file = self.read_installed();
        for id in ids {
            let dir = self.model_dir(id);
            if dir.exists() {
                fs::remove_dir_all(&dir)?;
            }
            let _ = fs::remove_file(self.root.join(".downloads").join(format!("{id}.zip.part")));
            file.models.retain(|m| &m.id != id);
        }
        self.write_installed(&file)
    }

    /// The other languages a freshly installed language's models also support, which are not
    /// active yet, so the UI can offer to activate them. Each model is offered only once: this
    /// marks `code`'s models as offered, so asking again returns nothing new.
    pub fn activation_offer(&self, catalog: &Catalog, code: &str) -> Result<Vec<String>> {
        let _guard = INSTALLED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut file = self.read_installed();
        let mine: Vec<String> = catalog.language(code).map(|l| l.models.clone()).unwrap_or_default();
        let fresh: Vec<String> = file.models.iter().filter(|m| mine.contains(&m.id) && !file.offered_models.contains(&m.id)).map(|m| m.id.clone()).collect();
        let on_disk = |id: &String| file.models.iter().any(|m| &m.id == id) && self.model_dir(id).join("model.bin").exists();
        let others: Vec<String> = catalog
            .languages
            .iter()
            .filter(|l| l.code != code && !file.languages.contains(&l.code))
            .filter(|l| l.models.iter().any(|m| fresh.contains(m)) && l.models.iter().all(on_disk))
            .map(|l| l.code.clone())
            .collect();
        for id in &mine {
            if !file.offered_models.contains(id) {
                file.offered_models.push(id.clone());
            }
        }
        self.write_installed(&file)?;
        Ok(others)
    }
}

/// Per-model step, before it is folded into a language-wide [`Step`].
#[derive(Debug, Clone, Copy)]
pub enum StepKind {
    Downloading(u64, u64),
    Verifying,
    Extracting,
}

fn extract_zip(archive: &Path, into: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    fs::create_dir_all(into)?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let relative = entry.enclosed_name().ok_or_else(|| ModelError::InvalidArchive(format!("unsafe path {}", entry.name())))?;
        let out = into.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            std::io::copy(&mut entry, &mut File::create(&out)?)?;
        }
    }
    if !into.join("model.bin").exists() {
        return Err(ModelError::InvalidArchive("model.bin is missing".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dawn-models-test-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::SeqCst)));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    type Archive = (PathBuf, String, u64);

    /// A tiny fake model archive; returns (path, sha256, size).
    fn make_archive(dir: &Path, name: &str, with_model_bin: bool) -> Archive {
        let path = dir.join(format!("{name}.zip"));
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        if with_model_bin {
            zip.start_file("model.bin", options).unwrap();
            zip.write_all(b"fake weights").unwrap();
        }
        zip.start_file("source.spm", options).unwrap();
        zip.write_all(b"spm").unwrap();
        zip.finish().unwrap();
        let sha = sha256_file(&path).unwrap();
        let size = fs::metadata(&path).unwrap().len();
        (path, sha, size)
    }

    fn model(id: &str, sources: &[&str], targets: &[&str], archive: &Archive) -> Model {
        Model {
            id: id.into(),
            sources: sources.iter().map(|s| s.to_string()).collect(),
            targets: targets.iter().map(|s| s.to_string()).collect(),
            url: format!("file:///{}", archive.0.to_string_lossy().replace('\\', "/")),
            size_bytes: archive.2,
            sha256: archive.1.clone(),
            license: "CC-BY-4.0".into(),
            attribution: "test".into(),
            prefix: None,
            target_tokens: Default::default(),
        }
    }

    /// `languages`: (code, ids of its models).
    fn catalog_with(models: Vec<Model>, languages: Vec<(&str, Vec<&str>)>) -> Catalog {
        Catalog {
            models,
            languages: languages
                .into_iter()
                .map(|(code, ids)| crate::catalog::Language {
                    code: code.into(),
                    name: code.into(),
                    native_name: code.into(),
                    beta: false,
                    models: ids.into_iter().map(String::from).collect(),
                })
                .collect(),
            ..Catalog::default()
        }
    }

    fn opts(cancel: &AtomicBool) -> InstallOptions<'_> {
        InstallOptions { proxy: None, cancel }
    }

    /// German (two own models) plus Urdu and Hindi sharing one multilingual model and one reverse model.
    struct World {
        store: ModelStore,
        catalog: Catalog,
        cancel: AtomicBool,
        shared_size: u64,
    }

    fn world() -> World {
        let dir = temp_dir();
        let (a, b, c, d) = (make_archive(&dir, "en-de", true), make_archive(&dir, "de-en", true), make_archive(&dir, "inc", true), make_archive(&dir, "inc-en", true));
        let catalog = catalog_with(
            vec![
                model("en-de", &["en"], &["de"], &a),
                model("de-en", &["de"], &["en"], &b),
                model("inc", &["en", "de"], &["ur", "hi"], &c),
                model("inc-en", &["ur", "hi"], &["en"], &d),
            ],
            vec![("de", vec!["en-de", "de-en"]), ("ur", vec!["inc", "inc-en"]), ("hi", vec!["inc", "inc-en"])],
        );
        World { store: ModelStore::new(dir.join("store")), catalog, cancel: AtomicBool::new(false), shared_size: c.2 + d.2 }
    }

    impl World {
        fn install(&self, code: &str) {
            self.store.install_language(&self.catalog, code, &opts(&self.cancel), &mut |_| {}).unwrap();
        }
    }

    #[test]
    fn checksum_of_known_content() {
        let dir = temp_dir();
        let path = dir.join("a.txt");
        fs::write(&path, b"abc").unwrap();
        assert_eq!(sha256_file(&path).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn language_is_installed_only_when_all_its_models_are() {
        let w = world();
        assert!(w.store.is_language_installed(&w.catalog, "en"), "hub is built in");
        assert!(!w.store.is_language_installed(&w.catalog, "de"));

        let first = w.catalog.model("en-de").unwrap();
        w.store.install_model(first, &opts(&w.cancel), &mut |_| {}).unwrap();
        assert!(!w.store.is_language_installed(&w.catalog, "de"), "one model is not enough");

        w.install("de");
        assert!(w.store.is_language_installed(&w.catalog, "de"));
        assert!(w.store.model_dir("en-de").join("model.bin").exists());
        assert_eq!(w.store.installed_models().len(), 2);
    }

    #[test]
    fn checksum_mismatch_deletes_the_download() {
        let dir = temp_dir();
        let archive = make_archive(&dir, "en-de", true);
        let mut bad = model("en-de", &["en"], &["de"], &archive);
        bad.sha256 = "0".repeat(64);
        let store = ModelStore::new(dir.join("store"));
        let cancel = AtomicBool::new(false);

        let err = store.install_model(&bad, &opts(&cancel), &mut |_| {}).unwrap_err();
        assert_eq!(err.code(), "checksum");
        assert!(!store.root().join(".downloads").join("en-de.zip.part").exists());
        assert!(!store.is_installed("en-de"));
    }

    #[test]
    fn archive_without_model_bin_is_rejected() {
        let dir = temp_dir();
        let archive = make_archive(&dir, "broken", false);
        let store = ModelStore::new(dir.join("store"));
        let cancel = AtomicBool::new(false);
        let err = store.install_model(&model("broken", &["en"], &["de"], &archive), &opts(&cancel), &mut |_| {}).unwrap_err();
        assert_eq!(err.code(), "invalid_archive");
        assert!(!store.model_dir("broken").exists());
        assert!(!store.root().join(".tmp-broken").exists());
    }

    #[test]
    fn unpublished_models_are_refused() {
        let dir = temp_dir();
        let archive = make_archive(&dir, "x", true);
        let mut m = model("x", &["en"], &["de"], &archive);
        m.url = "TBD".into();
        let store = ModelStore::new(dir.join("store"));
        let cancel = AtomicBool::new(false);
        assert_eq!(store.install_model(&m, &opts(&cancel), &mut |_| {}).unwrap_err().code(), "not_published");
    }

    #[test]
    fn cancelled_download_keeps_partial_file_for_resume() {
        let dir = temp_dir();
        let archive = make_archive(&dir, "en-de", true);
        let m = model("en-de", &["en"], &["de"], &archive);
        let store = ModelStore::new(dir.join("store"));
        let cancel = AtomicBool::new(true);
        let err = store.install_model(&m, &opts(&cancel), &mut |_| {}).unwrap_err();
        assert_eq!(err.code(), "cancelled");
        assert!(store.root().join(".downloads").join("en-de.zip.part").exists());

        cancel.store(false, Ordering::SeqCst);
        store.install_model(&m, &opts(&cancel), &mut |_| {}).unwrap();
        assert!(store.is_installed("en-de"));
    }

    #[test]
    fn a_shared_model_is_downloaded_once() {
        let w = world();
        w.install("ur");
        assert_eq!(w.store.installed_models().len(), 2);

        // Hindi needs the same two models: nothing to download, and it counts as installed at once.
        assert!(w.store.missing_models(&w.catalog, "hi").is_empty());
        let mut downloads = 0;
        w.store
            .install_language(&w.catalog, "hi", &opts(&w.cancel), &mut |step| {
                if let Step::Downloading { total, .. } = step {
                    assert_eq!(total, 0);
                    downloads += 1;
                }
            })
            .unwrap();
        assert_eq!(downloads, 1);
        assert_eq!(w.store.installed_models().len(), 2, "no second copy");
        assert!(w.store.is_language_installed(&w.catalog, "hi"));
    }

    #[test]
    fn models_installed_for_another_language_do_not_make_a_language_installed() {
        let w = world();
        w.install("ur");
        assert!(w.store.is_language_installed(&w.catalog, "ur"));
        assert!(!w.store.is_language_installed(&w.catalog, "hi"), "Hindi only counts once it was installed itself");

        // The choice is remembered in installed.json, which every Second Dawn app shares.
        let reopened = ModelStore::new(w.store.root());
        assert!(reopened.is_language_installed(&w.catalog, "ur"));
        assert!(!reopened.is_language_installed(&w.catalog, "hi"));
        reopened.install_language(&w.catalog, "hi", &opts(&w.cancel), &mut |_| {}).unwrap();
        assert!(w.store.is_language_installed(&w.catalog, "hi"));
    }

    #[test]
    fn sizes_account_for_what_is_already_installed() {
        let w = world();
        let before = w.store.language_sizes(&w.catalog, "hi");
        assert_eq!((before.download, before.total, before.freeable), (w.shared_size, w.shared_size, 0));

        w.install("ur");
        let hi = w.store.language_sizes(&w.catalog, "hi");
        assert_eq!(hi.download, 0, "Hindi adds nothing once Urdu is installed");
        assert_eq!(hi.total, w.shared_size);
        let ur = w.store.language_sizes(&w.catalog, "ur");
        assert_eq!(ur.freeable, w.shared_size, "Urdu is the only installed user of the shared models");

        w.install("hi");
        assert_eq!(w.store.language_sizes(&w.catalog, "ur").freeable, 0, "now Hindi needs them too");
        assert_eq!(w.store.language_sizes(&w.catalog, "hi").freeable, 0);
    }

    #[test]
    fn shared_models_stay_until_no_installed_language_needs_them() {
        let w = world();
        w.install("ur");
        w.install("hi");

        assert!(w.store.uninstall_language(&w.catalog, "ur").unwrap().is_empty(), "Hindi still needs everything");
        assert!(w.store.is_language_installed(&w.catalog, "hi"));

        let removed = w.store.uninstall_language(&w.catalog, "hi").unwrap();
        assert_eq!(removed.len(), 2);
        assert!(w.store.installed_models().is_empty());
        assert!(!w.store.model_dir("inc").exists());
    }

    #[test]
    fn uninstalling_one_language_keeps_unrelated_ones() {
        let w = world();
        w.install("de");
        w.install("ur");
        let removed = w.store.uninstall_language(&w.catalog, "de").unwrap();
        assert_eq!(removed.len(), 2);
        assert!(w.store.is_language_installed(&w.catalog, "ur"));
        assert!(!w.store.is_language_installed(&w.catalog, "de"));
    }

    #[test]
    fn finds_direct_models_for_pairs_that_would_otherwise_pivot() {
        let w = world();
        w.install("de");
        w.install("ur");
        // German to Urdu: the multilingual model reads German, so no pivot through English.
        let (m, dir) = w.store.find_model(&w.catalog, "de", "ur").unwrap();
        assert_eq!(m.id, "inc");
        assert_eq!(dir, w.store.model_dir("inc"));
        assert_eq!(w.store.find_model(&w.catalog, "en", "ur").unwrap().0.id, "inc");
        assert_eq!(w.store.find_model(&w.catalog, "ur", "en").unwrap().0.id, "inc-en");
        assert_eq!(w.store.find_model(&w.catalog, "en", "de").unwrap().0.id, "en-de");
        assert!(w.store.find_model(&w.catalog, "ur", "de").is_none(), "that pair needs the pivot");
    }

    #[test]
    fn language_progress_is_aggregated_over_the_missing_models() {
        let w = world();
        let mut last = (0, 0);
        w.store
            .install_language(&w.catalog, "ur", &opts(&w.cancel), &mut |step| {
                if let Step::Downloading { done, total } = step {
                    assert!(done >= last.0, "progress never goes backwards");
                    last = (done, total);
                }
            })
            .unwrap();
        assert_eq!(last.0, last.1);
        assert_eq!(last.1, w.shared_size);
    }

    #[test]
    fn active_languages_are_remembered_in_installed_json() {
        let w = world();
        w.install("ur"); // downloads the shared models and activates Urdu
        assert!(!w.store.is_language_installed(&w.catalog, "hi"));
        assert!(w.store.is_language_available(&w.catalog, "hi"), "its models are on disk");

        w.store.activate_language(&w.catalog, "hi").unwrap();
        let reopened = ModelStore::new(w.store.root());
        assert_eq!(reopened.active_languages(), vec!["ur".to_string(), "hi".to_string()]);

        reopened.deactivate_language(&w.catalog, "hi").unwrap();
        assert_eq!(ModelStore::new(w.store.root()).active_languages(), vec!["ur".to_string()]);
        // Deactivating only hides the language: its files stay.
        assert!(w.store.is_language_available(&w.catalog, "hi"));
        assert_eq!(w.store.installed_models().len(), 2);
    }

    #[test]
    fn a_language_without_its_files_cannot_be_activated() {
        let w = world();
        let err = w.store.activate_language(&w.catalog, "de").unwrap_err();
        assert_eq!(err.code(), "not_available");
        assert_eq!(w.store.activate_language(&w.catalog, "xx").unwrap_err().code(), "unknown_language");
    }

    #[test]
    fn only_the_last_user_of_a_model_makes_it_deletable() {
        let w = world();
        w.install("ur");
        w.store.activate_language(&w.catalog, "hi").unwrap();

        // Urdu and Hindi both use the models: deactivating one frees nothing.
        assert!(w.store.deactivate_language(&w.catalog, "ur").unwrap().is_empty());
        assert!(w.store.orphaned_models(&w.catalog).is_empty());

        // Deactivating the last user makes both models deletable.
        let orphans = w.store.deactivate_language(&w.catalog, "hi").unwrap();
        assert_eq!(orphans.len(), 2);
        assert_eq!(w.store.orphaned_models(&w.catalog).len(), 2);

        // Re-activating one of them takes the models out of the orphan list again, and deleting
        // an id that is needed again does nothing.
        w.store.activate_language(&w.catalog, "ur").unwrap();
        assert_eq!(w.store.delete_orphaned(&w.catalog, &orphans).unwrap(), 0);
        assert!(w.store.is_language_installed(&w.catalog, "ur"));

        w.store.deactivate_language(&w.catalog, "ur").unwrap();
        assert_eq!(w.store.delete_orphaned(&w.catalog, &orphans).unwrap(), w.shared_size);
        assert!(w.store.installed_models().is_empty());
        assert!(!w.store.is_language_available(&w.catalog, "ur"));
    }

    #[test]
    fn the_activation_offer_is_made_once_per_model() {
        let w = world();
        w.install("ur");
        // Right after the download: Hindi shares the models and is not active.
        assert_eq!(w.store.activation_offer(&w.catalog, "ur").unwrap(), vec!["hi".to_string()]);
        // Never again for these models, however often it is asked.
        assert!(w.store.activation_offer(&w.catalog, "ur").unwrap().is_empty());
        assert!(w.store.activation_offer(&w.catalog, "hi").unwrap().is_empty());
        // The "offered" mark survives reopening the store and a repair re-install.
        assert!(ModelStore::new(w.store.root()).activation_offer(&w.catalog, "ur").unwrap().is_empty());
        w.store.delete_language_files(&w.catalog, "ur").unwrap();
        w.install("ur");
        assert!(w.store.activation_offer(&w.catalog, "ur").unwrap().is_empty());
    }

    #[test]
    fn no_offer_when_nothing_is_shared_or_everything_is_active() {
        let w = world();
        w.install("de");
        assert!(w.store.activation_offer(&w.catalog, "de").unwrap().is_empty(), "German shares nothing");

        let w = world();
        w.install("ur");
        w.store.activate_language(&w.catalog, "hi").unwrap();
        assert!(w.store.activation_offer(&w.catalog, "ur").unwrap().is_empty(), "Hindi is active already");
    }

    #[test]
    fn unknown_language_is_an_error() {
        let store = ModelStore::new(temp_dir());
        let cancel = AtomicBool::new(false);
        let err = store.install_language(&Catalog::default(), "xx", &opts(&cancel), &mut |_| {}).unwrap_err();
        assert_eq!(err.code(), "unknown_language");
    }
}
