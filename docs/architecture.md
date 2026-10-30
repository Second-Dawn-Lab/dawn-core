# Architecture

Second Dawn is split across several repositories that sit **side by side** in one folder:

```text
SecondDawn/
  dawn-core/           this repo: shared Rust crates, shared TypeScript packages, model catalog, tools
  owll/                Owl-L, the offline translator
  dawn-app-template/   starter for new apps
  .github/             organization profile
```

Apps depend on `dawn-core` by **relative path** (`../dawn-core/...`), so a change in a shared crate or package is picked up immediately by every app checked out next to it. CI and release builds check out `dawn-core` at the commit recorded in the app's `dawn-core.ref` file, so they stay reproducible.

`dawn-core` itself holds four kinds of code:

- **`crates/`**: shared Rust crates, a Cargo workspace. Each app's Tauri backend depends on them by path.
- **`packages/`**: shared TypeScript/React packages (`@seconddawn/ui`, `@seconddawn/i18n`, `@seconddawn/settings`, `@seconddawn/updater`), a pnpm workspace. Apps depend on them with `link:` dependencies and import the TypeScript sources directly (no build step).
- **`models/`** and **`registry/`**: the catalog of downloadable model packs and the list of apps that Nest will show.
- **`tools/`**: developer tooling that is not shipped, e.g. `tools/convert-models` (turns Opus-MT models into language packs).

Apps (Tauri 2 + React + TypeScript + Vite) live in their own repositories. Apps follow a bird naming theme: Owl-L is the first, and the future launcher is called **Nest**.

## Rust crates

| Crate | Role |
|---|---|
| `dawn-core` | Paths (`%AppData%/SecondDawn`), atomic JSON read/write |
| `dawn-models` | Language catalog, resumable downloads, checksum verification, the shared model store. **The only shared code allowed to use the network** (pack downloads). The one other network use is an app's own update check, e.g. Owl-L asking `latest.json` on its GitHub release at most once a day |
| `dawn-translate` | `TranslationEngine` trait; `Ct2Engine` (CTranslate2 via `ct2rs`, oneDNN backend), the hub-and-pivot router with streaming batches and a per-sentence cache, language-aware sentence splitting, language detection (`whatlang`), plus a `StubEngine` for tests |
| `dawn-ocr` | OCR engines behind one trait (Windows OCR first) |
| `dawn-capture` | Screen capture and area-selection coordinate mapping |
| `dawn-hotkeys` | Double-press global shortcuts such as Ctrl+C+C (see [hotkeys.md](hotkeys.md)) |

## Shared model store

Models are never committed. `models/catalog.json` lists **languages**, each with the two models (`en->X`, `X->en`) it needs. `dawn-models` downloads, verifies and extracts them into a store shared by all apps, so a model is downloaded once:

- Windows: `%AppData%/SecondDawn/models` (with `installed.json`)

English is the hub; pairs without a direct model are translated through English. Details in [models.md](models.md).

## How a translation flows (Owl-L)

```text
React UI  --invoke-->  Tauri command (owll/src-tauri)  -->  Ct2Engine
   translate(text, from, to)                                  Router: plan(from, to)
   detect_language(text)                                        same language  -> input
                                                                direct model   -> 1 pass
                                                                otherwise      -> from -> en -> to
                                                              Ct2Runner: lazy-loaded models,
                                                              LRU (4 models), idle unload,
                                                              batches of 4 sentences, cancel flag,
                                                              per-sentence cache; each batch is
                                                              streamed to the UI (translate-progress)
```

The frontend only talks to Tauri commands (`owll/src/ipc.ts`). A new request cancels the previous one (latest keystroke wins). Language installs run on a background thread and report `pack-progress` events.

## Nest and the registry

`registry/apps.json` lists the apps Nest shows. Nest installs and updates apps, and apps use `@seconddawn/updater` to talk to it. Both are placeholders for now.

## Adding a new app

Use the [`dawn-app-template`](https://github.com/Second-Dawn-Lab/dawn-app-template) repository as a GitHub template, then add an entry to `registry/apps.json` in this repo. Reuse shared code from `crates/` and `packages/` rather than duplicating it.

Speed, backends and CPU requirements: [performance.md](performance.md).
