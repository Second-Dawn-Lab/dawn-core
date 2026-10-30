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
2. The page finds the clicked word and asks `alternatives(from, to, input, sentence, wordStart, currentWord)`: the whole
   translated *sentence* and the UTF-16 offset where the word starts.
3. **The prefix is exactly the pieces before the word.** The whole sentence is encoded once with the model's `target.spm`
   and cut where the word starts, using the byte spans SentencePiece reports for every piece. A piece belongs to the
   prefix when it *ends* at or before the word (SentencePiece counts the space in front of a word to that word's own
   piece, so a start-based rule is one piece too long: that was the first version of this fix, caught by the regression
   test, which then offered words for the position *after* "erstaunt"). Words split into several pieces, apostrophes and
   words after punctuation are handled by the same rule.
4. CTranslate2 returns up to `limit + 6` continuations (`return_alternatives`, first-token probability of at least
   0.5 %). Each candidate becomes a **whole word**: all pieces up to the next piece that starts a word, without the
   punctuation SentencePiece glues to the last piece ("erstaunt." is shown as "erstaunt"). Its preview is the start of
   the continuation ("erstaunt. ...", "sprachlos, weil ...").
5. Cleaning, in this order: fragments dropped (the first piece does not start a word although a prefix exists),
   punctuation-only candidates dropped, candidates that differ from one already listed only in case or punctuation dropped,
   and a word that merely repeats one of the **two words before the position** dropped. The current word comes first and
   is marked "current" (added from the sentence if the model did not return it); up to ten are listed, scrollable.
6. Choosing one shows that sentence and **pins** it: the router remembers `(from, to, source sentence) -> sentence`.
   Live re-translation while typing uses the pin for that sentence and translates everything else, so the choice
   survives until the source sentence changes. Pinned sentences are underlined with a dotted line. Pins are kept in
   memory (512 at most) and cleared with the translation cache.

Why scores are not used to rank or filter: CTranslate2 reports a score per hypothesis, but in alternatives mode it is
the score of the *generated continuation*, not of the alternative token, so "Ich bin **ich** erstaunt." scored better
(-1.0) than the natural "Ich bin erstaunt." (-2.2). The first-token probability floor and the repeat rule are the filters
that work.

Before and after, "I'm astonished." -> "Ich bin erstaunt.", clicking "erstaunt":

| | list |
|---|---|
| v0.2.0 | ich, erstaunt., überrascht., verblüfft., entsetzt., wunderlich., sehr, begeistert., mich, überglücklich. (punctuation inside words, "ich" and "mich" as if they were alternatives, the current word in the middle) |
| now | **erstaunt** (current), überrascht, verblüfft, entsetzt, wunderlich, sehr, begeistert, mich, überglücklich, each with a preview of how the sentence continues |

"sehr" ("Ich bin sehr erstaunt.") is a real alternative and stays. "mich" is the model's own, ungrammatical here
("Ich bin mich erstaunt"); it passes every rule that does not know German grammar and can be removed only with a
language-specific filter or a stricter probability floor (which also removes real alternatives).

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
