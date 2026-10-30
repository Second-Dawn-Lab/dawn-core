# Glossary

A glossary is a list of terms with the way they must be translated: `owl -> Eule` for English to German, or
`Kernel -> Kernel` to keep a name. It is applied to every translation by `dawn-translate` (`glossary.rs` and the
router), so text, the quick popup, captures and documents all use it.

Glossaries are **named lists** ("My business glossary", "Medical terms"); each has its own entries, and one can be
switched on at a time (or none). The Text tab, the Files tab and the quick popup share the choice (the setting
`glossaryActive`), so the popup uses the last one picked. A glossary file from before named lists is migrated into one list
called "My glossary", and stays switched on if the old global switch was on.

Entries belong to one language pair (`from`, `to`) and have three options: *match case*, *whole word* (default on)
and *enabled*. The whole glossary has a global switch. The app stores it as JSON; CSV import and export use the
columns `source,target,from,to,match_case,whole_word,enabled` (comma or, for Excel in many locales, semicolon
separated; the header and the last four columns are optional).

## The problem

A translation model cannot be told "use this word". It reads the whole sentence and writes the whole sentence.
Three ways to force a term were tried on the real English to German model (Opus-MT `en-de`, the same one the app
ships):

| Technique | What it does | Result |
|---|---|---|
| **(a) Placeholder** | replace the term by a token (`Zq1x`) before translation, put the target term in afterwards | The term is always there: 7 of 7 test sentences. The model copies the token like a name, but it has to *guess the grammar* around an unknown word. It chose the wrong article most of the time ("Der Eule", "dem Eule", "das Hund"). Placeholder shapes tried: `Zq1x`, `[[1]]`, `XYZ1`, `Zorbax` kept 7/7; `<1>` 5/7; `§1§` 6/7 (they get mangled). |
| **(b) Constrained decoding** (target prefix) | start the translation with a given prefix | Only works for the start of a sentence. Terms in the middle cannot be forced. Not usable. |
| **(c) Post-edit** | translate normally, then replace the model's own word for the term | Needs to know which output word belongs to the term (alignment); inflected forms ("Eulen", "der Eule") are not found by a plain search. Not reliable without an aligner. |
| **(d) Inline target term** | write the *target* term into the source sentence (`The Eule sleeps.`), the model copies it | The model knows many German words and then agrees the article with them: "Die Couch" (Couch), "Den Hund" (Hund), all correct. For a word it does not know well it is wrong too ("Das Eule"), or it translates the word away ("Kind" became "Art"). |

Measured articles over five sentences per term (placeholder vs. inline vs. the model's own translation):

| Term | placeholder | inline | model alone |
|---|---|---|---|
| owl -> Eule | 0/5 correct articles | 0/5 | 5/5 ("Die Eule") |
| dog -> Hund | 2/5 | 5/5 | 5/5 |
| child -> Kind | 4/5 | 2/5 (the term was translated away in 3: "Art") | 5/5 |
| sofa -> Couch | 0/5 | 5/5 | (uses "Sofa") |

## What was chosen

**Inline first, placeholder as the safety net**, per sentence:

1. If a sentence contains a glossary term and the translation is direct (no pivot through English), the target
   term is written into the source sentence and the sentence is translated. If every target term is present in
   the result (ignoring case), that is the answer.
2. Otherwise the terms are swapped for placeholders, the sentence is translated again, and each placeholder is
   replaced by its target term. A placeholder the model dropped is counted as missing; the model's own wording stays.
3. After a pivot (`de -> en -> ur`) only the placeholder is used: an Urdu word inside the intermediate English
   sentence would go into the second model.

Sentences without a term are translated exactly as before. The finished translation reports how many terms were put
in (`glossary_hits`); the Text tab shows a small "Glossary applied" marker.

## Limits (honest ones)

* **Grammar around the term can be wrong.** The target term is inserted as written. It is not declined, pluralized or
  re-cased, and the model may choose an article for a different gender ("Das Eule" for "die Eule"). The `inline` form
  is right for words the model knows; unknown words (rare nouns, new names) often get the wrong article. This is
  inherent to forcing a word into a model that has no such feature.
* **Inflected source forms do not match.** "owls" is not "owl" with *whole word* on. With it off, "owls" matches but
  the target is inserted unchanged ("Eule" instead of "Eulen"). Add the plural as its own entry.
* **Multi-word terms** work, and the longest match wins ("snowy owl" before "owl").
* **A term inside a longer proper name** is handled as the term.
* The glossary is per language pair; entries for `en -> de` are not used for `de -> en`.

If these limits matter, the next step is alignment-based post-editing (CTranslate2 can return attention for the
alignment): translate normally, find the output words aligned to the term, replace them. The model then keeps its
own grammar and only the word changes, but agreement with a differently-gendered target term still cannot be fixed
without a morphology tool.

## Tests

Unit tests (`glossary.rs`, `router.rs`) cover matching (case, whole word, overlaps, punctuation, multiple terms),
CSV round trips, protect/restore and the fallback order. `tests/features.rs` has the ignored evaluations quoted above
(`cargo test -p dawn-translate --test features -- --ignored --nocapture`).
