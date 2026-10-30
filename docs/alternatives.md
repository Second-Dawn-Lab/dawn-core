# Word alternatives

Click a word of a translation and Owl-L lists a few other words for that position. Choosing one rewrites the rest
of that sentence from the new word, so it stays grammatical ("Die Katze schläft" with "Der" gives "Der Kater schläft").
Other sentences are untouched.

## How it works

CTranslate2 can continue a translation from a given *target prefix* and return several different first tokens with
`return_alternatives` (each with the rest of the sentence generated from it). `ct2rs` 0.10 exposes this in
`ct2rs::sys::Translator::translate_batch_with_target_prefix` with `TranslationOptions::return_alternatives`,
`num_hypotheses` and `min_alternative_expansion_prob`. The high-level `ct2rs::Translator` only returns the best
hypothesis as text, so the engine loads a second copy of the model through the `sys` API, only when alternatives are
asked for (at most two such copies are kept, unloaded after five idle minutes).

For a click:

1. The translation of every sentence is returned with its text, its source sentence and what the last model stage
   read (`Segment`), and its position in UTF-16 units. A pivoted translation (`de -> en -> ur`) offers alternatives
   for the last stage only.
2. The page finds the clicked word, takes the translation of its sentence up to that word as the *prefix*, and asks
   `alternatives(from, to, input, prefix, current)`.
3. The prefix is encoded with the model's `target.spm`, the source sentence with its source tokenizer; the model returns
   up to seven continuations. The *word* of each is its first SentencePiece word (up to the next piece that starts with
   `▁`), the *text* is the whole new sentence. Duplicates, the current word and words the model gives under 0.3 % are
   dropped (this removes absurd suggestions: with no floor the list for "Katze" contained "Kacke"); five are shown.
4. Choosing one shows that sentence and **pins** it: the router remembers `(from, to, source sentence) -> sentence`.
   Live re-translation while typing uses the pin for that sentence and translates everything else, so the choice
   survives until the source sentence changes (it is a different key then). Pinned sentences are underlined with a
   dotted line. Pins are kept in memory (512 at most) and cleared with the translation cache.

Keyboard: Tab to the translation, Left/Right between words, Enter or Space opens the list, Up/Down choose, Enter
applies, Esc closes. Offsets are logical (not visual) positions, so right-to-left text is handled by the same code, but it has not been checked on screen.

## Measured

On the development machine (debug build of the app crates, release CTranslate2 with oneDNN, 4 threads, English to German
`opus-mt-en-de`, a 10 word sentence):

| | time |
|---|---|
| opening the list, first time (model load included) | about 2 s |
| opening the list, model loaded | about 0.25 to 0.85 s (longer when the sentence is long and there are many plausible words) |
| applying a choice | no extra model run: the alternative already contains the whole sentence; the pin is stored at once |

The beam is `count + 2` wide with the same number of hypotheses; smaller beams were faster but gave fewer, less varied
words. A word with only one plausible translation honestly shows "No other suggestion".

## Limits

* The list shows what the *model* considers plausible at that position, not a thesaurus. For function words it can
  offer a different sentence structure ("Da die Katze müde ist, ...").
* Alternatives are not offered while a translation is still streaming in, and not for a sentence that went through
  English in front of the clicked stage.
* A picked sentence is not re-checked against the glossary (the prefix already contains the glossary term; words the
  model generates after the pick are the model's own).
