# Quality mode (`dawn-llm`)

An optional local language model next to the fast translation engine (CTranslate2 / Opus-MT). It never replaces the fast
translation: that appears first, as before; the model rewrites it afterwards, sentence by sentence, and only when the user
turned Quality mode on. Off by default, nothing is loaded at app start, and nothing leaves the computer.

## What it does

| Feature | How |
|---|---|
| **Polish pass** | After the fast translation is complete and has rested 0.5 s, each sentence goes to the model with its source sentence and the fast draft: *"Rewrite the draft so that it is accurate, natural and idiomatic ..."*. The page gets the text after every sentence (`polish-progress`), so it changes quietly. |
| **Tone** | Formal / Informal / Automatic (`tone.rs`): one sentence in the same prompt ("Use formal address (Sie, Ihnen, Ihr)"). Offered for de, fr, es, it, pt, nl, pl, ru, uk, cs, tr, ja, ko, hi, ur (`tone_available`). |
| **Glossary with grammar** | The glossary terms of the *source* sentence are found also in inflected forms (`dawn_translate::find_forms`: "owls" for the term "owl", up to three added letters, longest term first) and go into the prompt: *"owls" (a form of the glossary term "owl") must be translated with "Eule" ... inflect it ... (a plural in the source needs a plural)*. The answer must still contain each term (`validate::term_present`: as written, or without its last letter, so "Eulen" contains "Eule"). If not, the answer is rejected and the fast translation, with the glossary applied as before, stays. |
| **Alternatives** | A click on a word asks the model for 8 other words for that position (`prompts::alternatives`), cleaned and deduplicated; the sentence is the old one with the word swapped. Any failure falls back to the CTranslate2 list. |
| **Documents** | `EngineTranslator::with_polish` polishes every translated text unit after the engine; the same cancel flag stops it. The editor has "Polish this segment" and "Polish all" (`quality_polish_text`). A model that cannot run ends the polishing for the rest of the document and says so (`doc-polish-stopped`). |
| **Write tab** | `WriteKind`: Improve, Shorter, More formal, More casual, Fix grammar. The answer is diffed word by word (`diff.rs`, LCS on words and whitespace); every change can be accepted or undone. |
| **Live captions** | Off by default; only in Calm speed and only when the model is already loaded (otherwise it loads in the background for the next line). 2.5 s budget per line, then the fast line stays. |

## Output is never trusted (`validate.rs`)

The prompt for each task is fixed (`prompts.rs`) and ends with "Only output ... nothing else". The answer is cleaned
(reasoning block, code fence, a label such as "Translation:", quotes around everything, trailing "Note:" lines) and then
rejected when it is empty, far too long or too short for the text it replaces, in the wrong language (script check for
non-Latin languages, `whatlang` for Latin ones), or drops a glossary term. A rejected answer is not an error for the user:
the fast translation stays.

## Backends (decision)

One trait, `Backend`, two implementations, both speaking the OpenAI-style `/v1/chat/completions` stream:

1. **Built-in: a `llama-server` sidecar** (llama.cpp, MIT), started on demand on a free port of `127.0.0.1`, one slot, no web UI,
   stopped after the idle time (default 5 min, a setting), when Quality mode is switched off, and when the app quits. On Windows it is put in a
   job object (`KILL_ON_JOB_CLOSE`), so it also dies when Owl-L crashes.
2. **Ollama**, if installed: `/api/tags` lists the user's models, the user picks one, nothing is downloaded or installed.

`Endpoint::parse` accepts only `127.0.0.1`, `localhost` and `[::1]` (a user name in front of the host, `https://`, other
addresses and `localhost.evil.example` are refused before a connection is made); the client uses no proxy and no redirects.

**Why a sidecar and not a Rust binding (`llama-cpp-2` / `llama-cpp-sys`).** A binding compiles llama.cpp from source inside
Owl-L, next to CTranslate2 (another large C++ build; this machine already needs 13+ minutes and several GB of RAM for that one), and puts about
the size of the runtime (the 25 MB zip below, 61 MB unpacked, most of it CPU variants) into every installer, also for the people who never turn
Quality mode on. The sidecar keeps the app unchanged for them, lets the runtime be updated without an app release, and separates the model's memory
(it is gone when the process ends). The price: a second process, a localhost port (bound to loopback only), and a runtime pack to download.

**Binary-size change of Owl-L.** No third-party crate was added: `dawn-llm` uses `ureq` (already built for `dawn-models`; the `json` feature only
pulls `serde_json`, already there), `whatlang` (already used by `dawn-translate`) and the `windows` bindings (already used by `dawn-hotkeys`, three more
feature flags). The crate is about 1,700 lines. A release build was not made to measure the exact number; expect well under half a megabyte.
The runtime is a separate download of **24.6 MB** (llama.cpp b12018, CPU, Windows x64; 61 MB unpacked).

## Packs

`models/catalog.json` has a new list `llms` (`dawn_models::LlmPack`): `"type": "llm"` (one GGUF) and `"type": "llm-runtime"` (a zip with
`llama-server.exe`). They use the same download, checksum and resume code as language packs but their own store (`%AppData%/SecondDawn/models/llm`), so removing a language never
touches them. Every url is pinned (a Hugging Face commit, a llama.cpp release tag) and has a sha256; a test checks that and that only
MIT and Apache-2.0 packs are listed.

All packs are mirrored, byte for byte, in **[Second-Dawn-Lab/owll-quality-packs](https://huggingface.co/Second-Dawn-Lab/owll-quality-packs)** (the model card there credits the
authors and links the sources; each license text is in `licenses/`). In the catalog the mirror, pinned to its commit, is the **first** url and the original repository
(Tencent, IndexTeam, Unsloth, the llama.cpp release) is the fallback, so a mirror that is down does not break Quality mode. File names, sizes and sha256 are the same
as the originals, and a test checks that every pack lists the mirror first and an original second.

License check before mirroring (the license files, not only the metadata): Hy-MT2 has a plain Apache-2.0 `LICENSE.txt`; Phi-4 mini's original repository has the MIT `LICENSE` and
`NOTICE.md` (the Unsloth GGUF repository has none); llama.cpp is MIT (the zip carries only the OpenMP license, so the llama.cpp `LICENSE` of tag b12018 is added next to it);
the IndexTeam Hugging Face repositories have an Apache-2.0 tag but **no license file**, the license is the root `LICENSE` (Apache-2.0) of the GitHub repository
`bilibili/Index-Translate`, whose README states it for the project.

To add a pack later: upload the file to the mirror repository, add its license text under `licenses/` and a row to its model card, then list the mirror url (with the new commit) before the original.

## Models evaluated

Test machine: Intel Core i5-9500T (6 cores, 2.2 GHz), 16 GB, no GPU, llama.cpp b12018 CPU, Q4_K_M, temperature 0. 20 English sentences into
de, fr, es, it, ru, zh, ja, ar, tr, ur, hi (220 translations per model; Phi-4 mini only 5 sentences per language), plus tone (de, fr, es, it, ru, formal and informal),
glossary inflection (owl → Eule in the plural, singular, with articles), polish of five stiff drafts, and Write (grammar fix, formal, shorter). There are no reference
translations, so there is no BLEU or COMET: "script ok" is automatic (the answer is in the language's script), the rest was read.

| Model | License | File | Memory (working set / private) | Speed | Notes |
|---|---|---|---|---|---|
| **Hy-MT2 1.8B** (Tencent) | Apache-2.0 (plain Apache text in `LICENSE.txt`) | 1.13 GB | 2.2 GB / 1.2 GB | 11 to 12 tokens/s; a polished short sentence in 1.0 to 1.7 s; model loaded in 2.2 s | 220/220 in the right script. Natural in every language of the set, also ur, hi, ar, tr. Tone right in de, fr, es, ru; Italian formal mixed "Potresti ... invitarla". Glossary: "Eulen", articles right. Wrong animal for "owls" in fr (chats noirs), tr (kurtlar), ur, hi. Fix grammar kept "me and him". **Default.** |
| **Index-Translate 2B** (bilibili) | Apache-2.0 | 1.31 GB | about 2.3 GB | 10.8 tokens/s | 220/220 script. Equal in general quality, better tone in es and it, fewer owl slips in fr and tr, bats for owls in ja and hi, "Соняки" in ru. **Translated a German text into English** in Write (language drift: the language check catches it). |
| **Qwen3 4B Instruct 2507** (Alibaba, GGUF by Unsloth) | Apache-2.0 | 2.50 GB | 5.0 GB / 2.6 GB | 5.7 to 7.3 tokens/s; 2.2 to 2.9 s per polished sentence | 220/220 script, but clearly the weakest translator here: Urdu degenerates into repeated words (8 s average, hit the token limit), Hindi, Turkish, Arabic and Italian have wrong or odd words ("Las avispas", "Gli struzzi"), "schläfern" for "schlafen". Tone fine. Not recommended. |
| **Phi-4 mini instruct** (Microsoft, GGUF by Unsloth) | MIT | 2.49 GB | 4.4 GB / 2.1 GB | 6 to 7.5 tokens/s; 1.3 to 2.5 s | Good in German and French (the other languages were not read in detail: 5 sentences each); poor in ar, ur, hi, tr (nonsense or wrong meaning), a wrong animal for "owls" in ja. The best **Write** result of the four on grammar ("Yesterday, he and I went ..."); its "more formal" answer added a letter template ("Dear [Recipient's Name]"). Listed as "Better" for European languages and rewriting, and never used for ar, tr, ur, hi (`LlmPack.languages`). |

Not tested: Hy-MT2 7B (Apache-2.0, Q4_K_M 4.6 GB, about 5.5 GB of memory; it did not fit the free memory of the test machine and the 2.5 to 3 GB class) and
Qwen3 1.7B (the 4B result made the family look unsuitable). **Excluded by license** without testing: Llama (community license), Gemma (Gemma terms), NLLB and other CC-BY-NC models.

**Decision.** *Small* = Hy-MT2 1.8B Q4_K_M, the default (1.13 GB, ~2.3 GB of memory). The brief asked for a "Better" model of 2.5 to 3 GB: no model of that class beat the 1.8B
translator on the language set; the dedicated translators win. Phi-4 mini is in the catalog as "Better" for rewriting and European languages (with its language list), Index-Translate 2B as
an "Alternative" of the same class for texts where Hy-MT2 stumbles. A real step up in translation quality is probably Hy-MT2 7B; it needs a decision (see the report).

## Speed and memory (Hy-MT2 1.8B, this machine)

| | |
|---|---|
| Process start to ready (`/health`), model file in the disk cache | 2.2 s (a cold disk read of 1.1 GB takes longer; not measured) |
| First polished sentence, model loaded, sentences of 8 to 12 words | 1.0 to 1.7 s (first token after 0.2 to 0.7 s) |
| Throughput | 11 to 12 tokens/s with 4 or 6 threads (no gain from 6) |
| Memory while loaded | 2.2 GB working set (1.1 GB of it the memory-mapped weights), 1.2 GB private |
| Idle | 0: the process is gone after the idle time |

GPU: not used. llama.cpp's Vulkan build (37.5 MB zip) exists, but it needs a second runtime pack and a driver check to be safe; Quality mode is CPU only for now.

## Memory rules (`memory.rs`)

The page shows the RAM the model needs, the RAM of the computer and what is free. Below about 16 GB installed a warning shows. A model is not loaded when less than
85 % of what it needs is free: `LlmError::LowMemory` becomes a message and the fast translation stays.

## Tests

Unit (`cargo test -p dawn-llm -p dawn-models`): tone availability, endpoint rules (local only), backend selection (off / built-in / Ollama), idle unload, memory checks, output cleaning and validation
(wrong language, extra text, missing term, length), prompts, the word diff, the SSE client against a local test server, pack install and catalog rules.
Integration (`#[ignore]`, needs a runtime and a model): `cargo test -p dawn-llm --test integration -- --ignored --nocapture --test-threads=1` with `DAWN_LLM_RUNTIME` and `DAWN_LLM_MODEL`:
polish, tone (German Sie / du), glossary inflection (owl → Eulen), Write "Fix grammar", alternatives.

## Changes after the first manual tests

* **One-click setup.** A model download brings the engine (llama.cpp runtime) with it when it is missing (`install_plan`: engine first, then the model; one cancel flag; progress per part). The engine is a small "Engine" line with its state. The status line names only what is missing (`status_state`: `off`, `not_set_up` + `model` / `engine` / `ollama_model`, `downloading`, `ready`, `loading`, `loaded`, `unavailable` + `ollama` / `ollama_models`); with the engine installed and no model it never asks for the engine.
* **Progress.** `quality_install` keeps the latest progress of every download (percent, bytes, a smoothed speed from `SpeedMeter`, time left computed from it), so a page opened during a download shows it at once (`QualityStatus.progress`).
* **Tiers.** `recommended` (Hy-MT2, the translation default), `alternative` (Index-Translate), `writing` (Phi-4 mini). Polishing uses the chosen translation model; the Write tab uses the writing model when it is installed and covers the language, else the translation model (`write_pack`); the page offers the download when it is missing. Models are never loaded together (the host swaps them, which is a reload).
* **Lifecycle.** `Llm` starts its backend through an injectable starter: after the idle unload, a crashed runtime or a failed load, the next request loads the model again (tests: `the_next_request_loads_the_model_again_after_the_idle_unload`, `a_failed_load_is_an_error_and_the_next_request_tries_again`, `a_runtime_that_stopped_answering_is_dropped_and_restarted`, and the real-model `the_model_loads_again_after_it_was_freed`). An observer tells the windows when the model starts loading, is loaded or is freed ("Loading model…"); a failure is a visible note with the reason and "Try again" in the result pane.
* **Tone cannot change the meaning** (`tone_check.rs`). Tone applies only when the source addresses someone as "you" (or the fast translation already addresses someone, as in an imperative); otherwise the normal polish is used. The tone prompt says what a tone must never do (change who speaks, make a question of a statement, add or remove information) and gives the example `My name is Ali and I work as a developer`. The answer is compared with the fast translation: a changed speaker (first-person words gone or new), a question mark that appears or disappears, or content words added or removed beyond the address forms reject it, and the sentence is polished once more without a tone. Tests include the reported sentence (`Ihr Name ist Ali, und Sie arbeiten …` and `Du heißt Ali und bist Softwareentwickler, oder?` are rejected) and the real-model `tone_never_changes_who_speaks_or_the_kind_of_sentence`.
* **Sentence alternatives** with Quality mode come from the model in the same tone and with the same glossary rules (`prompts::sentence_alternatives`), each checked like a polish; copies of the source, the current text and answers that break the tone rules are left out, and the list is hidden when nothing good is left (it never falls back to the translation model's sentences, which are lowercase, keep typos and ignore the tone).
* **Frontend state.** One shared status per window (`qualityStore`), refreshed on every backend change, when the window is used again, and when a polish result arrives; the tone picker shows a small lock (tooltip "Quality mode", click opens the settings) instead of a sentence when it is not usable. The settings store no longer lets a slow answer or an old announcement undo a newer local change (the swap bug), and the swap flips languages, text and the translator's state together.

## Second round: loops, readiness, writing model and messy input

* **No loops** (`dawn-core::degenerate`, used by `dawn-translate` and `dawn-llm`). The fast engine now caps the length of an answer from the length of its input (`max_decoding_for`: twice the longest input in characters plus 24, 48 to 512 tokens; the old cap was 512 for every sentence) and penalizes repeating a little (`repetition_penalty` 1.1). `find_degeneration(source, output)` finds a repeated word, a repeated group of two to four words, runs of one-letter junk, a short piece of letters repeated many times (also in text without spaces) and output far longer than the source, and returns the part before the loop. In the router (`run_guarded`) such an answer is asked again once with stricter decoding (`run_strict`: repetition penalty 1.5, no repeated 3-gram, beam at least 4, a shorter cap); if that fails too, the part before the loop is used (or the source sentence when less than two words are left) and the sentence is reported as `unclean` (`Segment.unclean`, the note "Couldn't translate this sentence cleanly"). A loop is never returned or cached. The same detector is part of `validate::check`, so a Quality mode answer that loops is rejected (`Reject::Degenerate`) and the fast translation stays. `tests/degeneration.rs` (real model, ignored) runs the reported sentence and four more garbled ones.
* **Readiness is "an installed engine plus an installed model that can do the task"** (`choose_model`, `choose_writer`, `Candidate`). The chosen model is used when it is installed and good in the language; otherwise the best installed one, in the order recommended, alternative, writing; the status shows which is in use and says so when the chosen one is gone. Phi-4 mini ("For writing") can be chosen for polishing; it is not used for Arabic, Turkish, Urdu and Hindi (`LlmPack.languages`), where the fast translation is shown with a note. The Write tab is ready with any installed model and prefers the writing model. Tests cover none / only recommended / only writing / both, and a model that does not cover the language.
* **The Write tab says which model produced the result** (small text in the result footer). Fix grammar now names the case of pronouns (`me and him went` becomes `he and I went`), subject-verb agreement, plurals, verb forms and tenses; the reported sentence gives `Yesterday he and I went to the shop and bought three apples.` with both Phi-4 mini and Hy-MT2 (`write_fix_grammar_fixes_the_pronoun_case_too`).
* **Messy input** (`prompts::translate_intended`). With Quality mode on, the model translates from the source itself ("The text may contain typos, missing words, words run together or speech-recognition errors: translate the meaning the writer intended, naturally and correctly, and do not add information that is not in the text"). The fast translation is only a hint, and it is left out when it is `unclean`, when it is hardly a translation, or when it is mostly the source's own words carried through (`usable_hint`: "im ali und ich arbeite als Softwar-Devloper"). All checks stay: target language, glossary terms, the tone rules (compared with the fast translation only when that is trustworthy, else only the kind of sentence), degeneration, length. Meeting-caption polish uses the same function. Real-model tests: `messy_input_is_translated_by_its_intended_meaning_not_word_by_word` (`hi how are you ? i m softwar devllper` gives `Hallo, wie geht's dir? Ich bin ein Software-Entwickler`, with and without a hint).
* **Latency**, a short sentence, Hy-MT2 1.8B loaded, this machine: rewriting the draft (before) 0.95 to 1.0 s, translating the intended meaning with the draft as a hint (after) 0.79 to 0.82 s (the new prompt asks for a translation, not a rewrite, and the answer is the same length); first call after loading 1.2 s.
* **"Did you mean ...?"** without Quality mode (`dawn-dict::spell`, command `spell_suggest`): built only on the key lists (headwords and forms) of an installed dictionary pack of the source language. A word is corrected only when it has at least four letters, is not in the dictionary, and exactly one headword with the same first letter is within one edit (five letters or fewer) or two (longer). Names (a capital inside a sentence), ALL CAPS, words with capitals inside, words with apostrophes or hyphens, and anything in a chunk with digits, `@`, `/`, `\`, `#`, `_`, `~`, `://` or a dot between letters (links, addresses, file names) are never touched. The suggestion shows the changed words in context and one click applies the corrected text.

## Third round: short texts, glossary state, minimal grammar fixes, alternatives

* **Short texts** (`validate::check`). The longest answer is now the ratio or the text plus about six words (50 characters), whichever is larger (not for "make it shorter", which never allows a longer answer), and a text under 40 characters has no lower bound. So `i and me go zoo` (15 characters) may become `He and I go to the zoo.` (23); the old Fix grammar limit of 1.5 times rejected it. The translation polish uses the same function. For a short text or answer (under 8 words) the statistical language detector is not used: the script is still checked, and an answer that reads as English (many English function words) for a non-English text is rejected unless the text itself read as English (its language label may have been guessed wrong from a few words). Every rejection has its own code (`Reject::code`, `quality_rejected_too_long`, ...), so the Write tab can say what went wrong; since the model gives the same answer to the same text, a rejection never suggests "Try again".
* **Fix grammar is the smallest change** (`tone_check::is_minimal_fix`). The prompt says never to change a noun, number, name, tense or content (with the owls example). The answer is rejected as `ChangedMeaning` when a noun after a number-neutral determiner ("the", "my", ...) changes number (`The owls is sleeping` must not become `The owl is sleeping`; a numeral such as "three apple" does force the plural, so that fix is allowed) or when more than a few content words are added or removed; the task then asks once more with a stricter prompt (`prompts::write_strict_fix`: only verb agreement and forms, pronoun case, spelling, punctuation). `The owls is sleeping in the barn.` gives `The owls are sleeping in the barn.`
* **Polished state** (`rewriteView`). A rewrite counts when the model's answer was used for at least one sentence, also when it came out identical to the fast translation. A glossary sentence is often already right in the fast pass ("Die Eulen jagen nachts im Wald.") and the model returns it unchanged; the result pane used to compare the texts, found no difference and dropped the marker, the toggle and the glossary highlight. The highlight comes from the rewrite's own term positions (`find_forms`: "Eulen" for "Eule").
* **Sentence alternatives keep the content** (`tone_check::content_preserved`). Each alternative is compared with the polished sentence: it may not lose more than a fifth of its content words, may not add more than a third, and names that stand in the source and in the polished sentence must stay. `Ich arbeite als Softwareentwickler` (without "Ich bin Ali") is dropped for `im ali and i work as softwar devloper`. Weak spot: a wrongly worded alternative that keeps all content words (a typo carried through, "Im Ali und ich arbeite …") still passes.

## English source clean-up (`dawn_core::normalize`)

A typo that happens to be a word of the target language is carried through by both engines: `im ali` became `Im Ali` ("Im" is "in the" in German), and no check can tell. `normalize_source` therefore cleans up English text before it is translated, in `Router::translate_streaming`, so it covers the fast engine, Quality mode (the polish sees the cleaned sentence as its source), the text tab, the popup, captions and documents. The text on screen is never changed; only what is sent to the engines.

* Only English sources. Only whole words. Conservative: `im` becomes `I'm` before a lowercase word, at the start of a text or sentence, or after hi/hello/and/so/...; `Im` at the start of a sentence only if no capitalised word follows (`Im Wald`, `Im Chang-min` stay); `i m` and a lone `i` become `I'm` and `I`; `dont`, `doesnt`, `cant`, `wont`, `isnt`, `arent`, `didnt`, `wasnt` get their apostrophe; a lone `u` becomes `you`, `ur` becomes `your` only before a noun such as name or email.
* Never touched: all-caps (`IM`), words with digits or inside URLs, e-mail addresses, paths, code, `u.s.`, `u-turn`, and glossary terms (matched as whole words).
* A fallback that shows the source because the engine failed shows the cleaned-up sentence.
* The polish prompt (`prompts::translate_intended`) also says that a typo in the source is not a word of the target language, with the `im Ali` example. Tests: unit tests in `normalize.rs` and `router.rs`, and the ignored real-model test `a_source_typo_that_is_a_german_word_is_not_copied_through`.
