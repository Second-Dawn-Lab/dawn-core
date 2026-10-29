# Translation engine and performance

All numbers were measured on one machine: Intel Core i5-9500T (6 cores, no hyper-threading, AVX2 but no AVX-512), 15.8 GB RAM, Windows 11, German to English with the Opus-MT int8 model. Run-to-run noise on this machine is large (identical runs differed by up to 2x when memory was tight), so read the numbers as orders of magnitude, not as promises.

## Backend: oneDNN on x86-64, ruy elsewhere

CTranslate2 needs an int8 matrix-multiply backend. The build picks one by CPU architecture, with no flags (target-specific dependencies in `crates/dawn-translate/Cargo.toml`):

| CPU | Backend | Notes |
|---|---|---|
| x86-64 (Windows, Linux, Intel Mac) | **oneDNN** | Built from source with CMake. |
| ARM64 (Windows on ARM, Apple Silicon, Linux ARM) | ruy | Small, and fast on ARM. Not built or tested here, only the dependency resolution was checked. |

To force ruy on x86-64 (for example if oneDNN does not build), change the `features` of the x86-64 `ct2rs` entry in that file from `dnnl` to `ruy`.

Measured, whole 1,492-character passage (12 sentences), beam 2, one call:

| Threads per model | ruy | oneDNN |
|---|---|---|
| 1 | 14.5 s | 2.0 s |
| 2 | 7.6 s | 1.4 s |
| 4 (default here) | 5.0 s | 1.1 s |
| 6 | 6.6 s | 1.1 s |

In the release app, one paste of that passage, final text on screen: about 3.4 s on a fresh start (which includes about 1 s to load the model), about 1.8 s afterwards, and 0.3 s (only the debounce) when the same text is pasted again. With ruy the same paste took about 5 s per pass.

| | ruy | oneDNN | Change |
|---|---|---|---|
| Release `owll.exe` | 17.0 MB | 48.9 MB | +31.9 MB |
| NSIS installer (`Owl-L_0.0.0_x64-setup.exe`) | not built | 8.0 MB | |
| Build, cold (CTranslate2 + oneDNN from source) | about 5.5 min | about 13 min | CI caches it, see below |

oneDNN is linked statically, so there are no extra DLLs. Translation quality is equivalent; the two backends round int8 differently, so about a third of sentences are worded slightly differently (60 sentences compared, 39 identical, none with a consistent loss of meaning).

## CPU requirements

- **x86-64 with AVX2 (Intel Haswell 2013 and later, AMD Excavator/Zen and later): full speed.** This is what the numbers above are from.
- **Older x86-64 CPUs (SSE4.1 or AVX only): they run and give the same output, but about 7x slower.** Measured by capping oneDNN (`ONEDNN_MAX_CPU_ISA`) and CTranslate2 (`CT2_FORCE_CPU_ISA`): one sentence took 1.1 s with AVX2, 8.6 s with AVX and 7.7 s with SSE4.1.
- **AVX-512** is detected and used at run time by both libraries. The test machine has none, and the AVX-512 path itself has not been tested here.
- 64-bit Windows 10 or later (WebView2). 4 GB RAM is enough for a few models: two loaded models use about 270 MB, three about 390 MB.

CTranslate2 prints what it picked when a model loads (`CPU: GenuineIntel (SSE4.1=true, AVX=true, AVX2=true, AVX512=false)`, `Selected ISA: AVX2`, `GEMM_S8 backend: DNNL`); run `cargo test -p dawn-translate --test quality -- --ignored --nocapture cpu_report` to see it.

## How text is translated

1. **Sentence splitting is language-aware** (`dawn-translate/src/split.rs`). A full stop after an abbreviation ("z. B.", "Dr.", "Mr.", "usw."), an initial ("U.S."), a number ("3.5", "1.000.000") or, for German, an ordinal ("3. Oktober") does not end a sentence. This matters for quality: cutting a sentence in half made the model invent text (`==References==`). To add a language, add a `Rules` entry in `rules_for`.
2. **Batches of 4 sentences** go to the model, and each finished batch is shown immediately, so the result pane fills from the top. A newer request cancels the older one between batches (the latest keystroke wins).
3. **A per-sentence cache** (model, beam, sentence) means editing one sentence only translates that sentence. The same text again costs nothing.
4. **Threads per model** default to about two thirds of the logical cores, at least two (4 on a 6-core CPU; 6 was no faster than 4). It can be changed in Settings, Advanced.
5. **Quick draft first** (Settings, Advanced; off by default) translates with beam 1 first, then replaces it with beam 2. It shows text sooner but does about twice the work, so it only makes sense on a slow CPU.

Beam size trade-off on the same passage (ruy numbers; oneDNN is about 4x faster across the board): beam 1 is about 30% faster than beam 2 but changes the wording of about 40% of sentences; beam 4 is about twice as slow as beam 2 and was no better.

## Measuring

- `DAWN_TRACE=1` makes the app log timestamps for every step of a request (input, debounce, detection, each batch, painting) to stderr. Run the release binary as `DAWN_TRACE=1 owll.exe 2> trace.txt`.
- `cargo test -p dawn-translate --test perf -- --ignored --nocapture --test-threads=1` runs the benchmarks on a stand-in passage and on a real one; `--test quality` dumps sample translations and the CPU report. Use `--no-default-features --features ruy` to measure ruy.
- All of them need the converted German model (see [models.md](models.md)).

## Known weak spots

- **The English to Urdu and English to Japanese models are poor** (they are marked "Beta" in the catalog and the UI shows a badge with a tooltip). With either backend they often produce fluent text that is not a translation of the input (Bible-style fragments in Japanese). German, French and the reverse directions look fine. Do not ship those two packs as they are.
- Pivot translation (for example German to Urdu) inherits the weaknesses of both steps.
- Free memory matters: with under 3 GB free the benchmarks became unreliable and a build ran out of heap.
