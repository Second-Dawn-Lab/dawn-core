//! Downloads a Quality mode pack from the bundled catalog through the real install path (network, so `#[ignore]`):
//!
//! ```text
//! set DAWN_PACK_ID=hy-mt2-1.8b-q4km
//! cargo test -p dawn-models --test llm_download -- --ignored --nocapture
//! ```
//!
//! Only the first url (the mirror) is used, so a pass proves the mirror serves the file with the catalog's size and sha256.

use std::sync::atomic::AtomicBool;

use dawn_models::{Catalog, InstallOptions, ModelStore};

#[test]
#[ignore = "downloads a pack"]
fn a_catalog_pack_installs_from_its_first_url() {
    let id = std::env::var("DAWN_PACK_ID").unwrap_or_else(|_| "llama-cpp-b12018-win-x64-cpu".into());
    let mut pack = Catalog::bundled().llm(&id).expect("pack in the catalog").clone();
    pack.urls.truncate(1);
    println!("downloading {} from {}", pack.id, pack.urls[0]);
    let dir = std::env::temp_dir().join(format!("dawn-llm-download-{}", std::process::id()));
    let store = ModelStore::new(&dir);
    let cancel = AtomicBool::new(false);
    let mut last = 0;
    store
        .install_llm(&pack, &InstallOptions { proxy: None, cancel: &cancel }, &mut |step| {
            if let dawn_models::StepKind::Downloading(done, _) = step {
                last = done;
            }
        })
        .expect("install");
    let file = store.llm_file(&pack).expect("installed file");
    let size = std::fs::metadata(&file).unwrap().len();
    println!("installed {} ({size} bytes, downloaded {last})", file.display());
    if !pack.is_runtime() {
        assert_eq!(size, pack.size_bytes);
    }
    std::fs::remove_dir_all(&dir).ok();
}
