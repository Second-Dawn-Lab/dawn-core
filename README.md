<p align="center">
  <img src="assets/brand/second-dawn-main.png" alt="Second Dawn" width="360" />
</p>

# dawn-core

Shared code for every [Second Dawn](https://github.com/Second-Dawn-Lab) app: free, offline, private apps.

`dawn-core` holds what apps would otherwise duplicate: the offline translation engine, the shared model store, OCR, screen capture, global hotkeys, and the design system and settings helpers for the React front ends. Apps live in their own repositories and use it from a sibling folder.

## What is inside

| Folder | Contents |
|---|---|
| `crates/` | Cargo workspace: `dawn-core` (paths, JSON), `dawn-models` (catalog, downloads, model store), `dawn-translate` (CTranslate2 engines, routing, language detection), `dawn-ocr`, `dawn-capture`, `dawn-hotkeys` |
| `packages/` | pnpm workspace: `@seconddawn/ui` (design system), `@seconddawn/i18n`, `@seconddawn/settings`, `@seconddawn/updater` |
| `models/catalog.json` | Catalog of downloadable language packs (weights are never committed) |
| `tools/convert-models/` | Python tooling that converts Opus-MT models into language packs |
| `registry/apps.json` | The apps Nest (the future launcher) will list |
| `docs/` | [architecture](docs/architecture.md), [models](docs/models.md), [performance](docs/performance.md), [detection](docs/detection.md), [OCR](docs/ocr.md), [hotkeys](docs/hotkeys.md) |

## How apps use it

Clone `dawn-core` **next to** the app:

```text
SecondDawn/
  dawn-core/
  owll/                 (or any other app)
```

- **Rust:** the app's `src-tauri/Cargo.toml` uses path dependencies, e.g. `dawn-translate = { path = "../../dawn-core/crates/dawn-translate" }`.
- **TypeScript:** the app's `package.json` uses `link:` dependencies, e.g. `"@seconddawn/ui": "link:../dawn-core/packages/ui"`. The packages ship TypeScript sources, so Vite compiles them with the app and hot reload works when you edit them.
- Run `pnpm install` in `dawn-core` too, so the shared packages find their own dependencies.
- Apps pin the `dawn-core` commit that CI and releases use in a `dawn-core.ref` file.

Data lives in `%AppData%/SecondDawn` (shared model store in `models/`, per-app data in `<app>/`).

## Build and test

Requirements: Node.js 20+, pnpm, Rust (stable). On Windows the translation engine ([CTranslate2](https://github.com/OpenNMT/CTranslate2) with oneDNN) is compiled from source on the first build (about 13 minutes, a few GB of RAM): install Visual Studio Build Tools (Desktop development with C++) and [CMake](https://cmake.org/download/) 3.18+ on your `PATH`. The static C runtime is configured in [`.cargo/config.toml`](.cargo/config.toml).

```bash
pnpm install
pnpm lint
pnpm typecheck
cargo check --workspace
cargo test --workspace
```

Speed, CPU requirements (AVX2 recommended) and the ruy alternative for non-x86: [docs/performance.md](docs/performance.md).

## Credits

Built on **OPUS-MT (Helsinki-NLP)**, **Mozilla Bergamot / Firefox Translations**, **CTranslate2**, **Tauri** and **RapidOCR**.

## Contributing

See [CONTRIBUTING](https://github.com/Second-Dawn-Lab/.github/blob/main/CONTRIBUTING.md).

## License

[MIT](LICENSE) © Second Dawn contributors.
