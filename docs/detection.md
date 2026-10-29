# Language detection

Owl-L detects the source language with [`whatlang`](https://crates.io/crates/whatlang) (tiny, statistical, no model).

## Rules

- **Long text** (4 or more words and 25 or more characters): all supported languages are candidates. A detected language that is not installed or active shows "<Language> detected · not installed" with Download or Activate. "Couldn't detect the language" appears only when the guess is weak, and only after typing has stopped.
- **Short text**: only the active languages plus English are candidates. A short list of very common words ("Hallo", "Danke", "Merci", "hola", ...) settles the obvious cases, a trusted statistical guess is used next, and a weak guess falls back to the last detected (or last chosen) source language. Short text is never reported as undetected.

The word list is a stopgap: `whatlang` is poor on single words (below).

## lingua vs whatlang

Evaluated in a scratch project, release build with LTO, languages de/en/fr/es/it/ur/ar. 31 single words, 30 short phrases, 7 passages of about 120 characters. Not adopted; waiting for a decision.

| | words | phrases | passages | time per detection (words/phrases/passages) |
| --- | --- | --- | --- | --- |
| whatlang, all languages | 6/31 | 19/30 | 7/7 | 82 / 98 / 126 µs |
| whatlang, restricted to the 7 | 18/31 | 26/30 | 7/7 | 12-22 / 14-18 / 29-32 µs |
| lingua low accuracy, the 7 | 24/31 | 28/30 | 7/7 | 43 / 54 / 290 µs |
| lingua high accuracy, the 7 | 27/31 | 30/30 | 7/7 | 41 / 69 / 331 µs |

("all languages" counts a detection outside the seven as wrong, which is why it looks so bad on single words.)

| | executable | peak memory |
| --- | --- | --- |
| whatlang only | 0.42 MB | 4.8 MB |
| + lingua, 7 languages (low or high accuracy, same binary) | 40.2 MB | 30.4 to 30.9 MB |

Lingua is clearly more accurate on short text, but costs about 40 MB of binary and 30 MB of memory for just seven languages (its models are compiled in per language). That is large for Owl-L. Even high accuracy missed "Hallo", "hello", "Merci" and "ventana" among the seven. Options for a decision: keep `whatlang` plus the word list; or load lingua only for short text; or build a small language-ID of our own from the translation models' vocabularies.
