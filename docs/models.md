# Models and language packs

Second Dawn apps translate offline with [Opus-MT](https://github.com/Helsinki-NLP/Opus-MT) models (Helsinki-NLP), converted to [CTranslate2](https://github.com/OpenNMT/CTranslate2) int8 and run from Rust. Model files are **never committed**; they are downloaded into a store shared by all Second Dawn apps.

## The hub design

Users pick **languages**, not language pairs. What gets downloaded is **models**, and models and languages are linked through the catalog (format below).

- **English is the hub.** It is built in and always available.
- **A language lists the models it needs.** A plain Opus-MT language such as German needs two (`en->de`, `de->en`, about 130 MB together). Languages served by a multilingual model share it: Urdu, Hindi, Punjabi and Bengali all use the same two models (about 445 MB once, not four times).
- **A model serves every pair of its source and target languages.** The router uses the most specialized installed model that serves the pair, so with Urdu active German to Urdu is translated directly by the multilingual model. Only pairs no model serves, such as Urdu to German, go through English (`X -> en -> Y`) and are marked "via English" in the UI.
- **Active / inactive.** A language is *active* when it is shown in the language pickers. If its models are already on disk because another language needs them, it appears on the Language packs page with an Active/Inactive switch ("Included with Urdu") instead of a Download button, and switching it on is instant. Right after a download that also makes other languages available, a dialog offers to activate them (unticked by default, shown once per model).
- **Hiding keeps the files.** Deactivating a language only hides it. When the last active language that uses a model is deactivated, Owl-L offers to delete the files and shows the space that frees; deleting is never automatic.

## Catalog format (`models/catalog.json`, version 2)

Models and languages are listed separately, so several languages can use the same model:

```json
{
  "version": 2,
  "hub": "en",
  "models": [
    { "id": "opus-mt-en-de-ct2-int8", "sources": ["en"], "targets": ["de"],
      "url": "https://example.org/opus-mt-en-de-ct2-int8.zip", "sizeBytes": 62910771, "sha256": "d47a029d...",
      "license": "CC-BY-4.0", "attribution": "Helsinki-NLP / OPUS-MT" },
    { "id": "opus-mt-tc-bible-big-inc-ct2-int8", "sources": ["en", "de", "fr", "es", "pt"], "targets": ["ur", "hi", "pa", "bn"],
      "targetTokens": { "ur": ">>urd<<", "hi": ">>hin<<", "pa": ">>pan<<", "bn": ">>ben<<" }, "...": "..." }
  ],
  "languages": [
    { "code": "de", "name": "German", "nativeName": "Deutsch", "models": ["opus-mt-en-de-ct2-int8", "opus-mt-de-en-ct2-int8"] },
    { "code": "ur", "name": "Urdu", "nativeName": "اردو", "beta": true, "models": ["opus-mt-tc-bible-big-inc-ct2-int8", "opus-mt-tc-bible-big-inc-en-ct2-int8"] },
    { "code": "hi", "name": "Hindi", "nativeName": "हिन्दी", "beta": true, "models": ["opus-mt-tc-bible-big-inc-ct2-int8", "opus-mt-tc-bible-big-inc-en-ct2-int8"] }
  ]
}
```

- A **model** serves every pair `source -> target` of its `sources` and `targets`. A plain Opus-MT model has one of each; a multilingual one has many.
- A **language** lists the models it needs (ids). Urdu, Hindi, Punjabi and Bengali list the same two models, which are downloaded once. Installing the next of them costs no download ("No extra download" in the UI), and removing one keeps the models until no installed language needs them.
- `installed.json` records which languages are active as well as which models exist; having a model on disk because another language needs it does not make a language active.
- **Direct pairs:** the router picks the most specialized installed model that serves the pair, so German to Urdu is translated directly by the multilingual model; only pairs no model serves (for example Urdu to German) go through English.
- `targetTokens` (or `prefix` for a single-target model) is the token that selects the target language, put in front of every source sentence as **one vocabulary token**. If the token is not in the model's vocabulary it is passed as plain text (older Opus-MT models such as English to Arabic).
- `url` is `https://` or `file://`. `sha256` is checked before anything is installed; a mismatch deletes the download and shows an error.
- `beta` (optional, on a language) marks a language whose translations are known to be unreliable (a "Beta" badge with the tooltip "Translation quality may be unreliable").

The catalog is compiled into the app (`include_str!`), so rebuild after editing it. Older version 1 catalogs (models inline in each language) are migrated by `tools/convert-models/catalog_tools.py`.

## Converting models

`tools/convert-models/convert.py` does everything in one go for the plain two-model languages; `package_multi.py` packages a multilingual model and adds it to the catalog (see its header). `convert.py` (see its [README](../tools/convert-models/README.md)):

1. fetches `Helsinki-NLP/opus-mt-en-<x>` and `opus-mt-<x>-en` from Hugging Face (and reports a language whose direction is missing instead of guessing a substitute),
2. converts each with `ct2-transformers-converter --quantization int8`, keeping the SentencePiece files,
3. packages each model as a zip with a `manifest.json`,
4. computes the size and sha256 and prints (or merges) `catalog.json` entries.

Converted files go to `tools/convert-models/out/`, which is git-ignored.

## Licensing and attribution

Opus-MT models are released by the Helsinki-NLP group under **CC-BY-4.0** or **Apache-2.0** (the converter reads the license from each model card and writes it to the catalog). Both require attribution; the language packs page shows the license and "Helsinki-NLP / OPUS-MT" for every language, and each archive carries a `manifest.json` with the source repository. The conversion to CTranslate2 int8 does not change the license of the weights. Check a model's license before redistributing it.

## Network rule

`dawn-models` is the only crate allowed to touch the network. Translation, detection and everything else run without any connection.
