# Offline dictionary: data decision (B1)

Status: **decided on paper, not built.** The Dictionary tab in the word popup still says "Coming soon". This page is what
was found and what has to be settled before the pack and the engine are written.

## What a dictionary pack needs

Headword, part of speech, ranked translations, short examples, German gender/plural and verb forms, and **inflected
forms to base forms** ("erstaunt" to "erstaunen", "Häuser" to "Haus", "ging" to "gehen"). The last one decides the
choice: a plain word list cannot do it.

## Sources checked

| | Wiktionary through kaikki.org (wiktextract) | FreeDict (deu-eng, eng-deu) |
|---|---|---|
| Origin | Wiktionary, parsed into JSON lines by `wiktextract` | the `ding` database of TU Chemnitz (Frank Richter), converted to TEI |
| License | Wiktionary's text is **CC BY-SA** (4.0, and GFDL as a second option). The kaikki.org page itself states no license and asks for a citation of the Wiktextract paper and a link back | **GPL-2+** according to Debian's copyright file for the package (`deu-eng/*`, "FreeDict contributors"); the AUR package lists GPL/AGPL3. The upstream license text inside the archives was not read |
| Duties | credit Wiktionary and its editors with a link; the pack stays CC BY-SA (share-alike). The app code is a separate work and stays MIT | GPL: the pack would have to be offered under the GPL with its source; mixing with an MIT app is arguable and not worth the risk |
| Coverage | the English edition has entries for words of almost every language **with English glosses**: German to English from the German entries, English to German from the `translations` lists of English entries. Other languages vary a lot in size | 517,534 German to English and 460,315 English to German headwords; many other pairs, mostly small (French to German 48,578; English to Russian 62,181) |
| Entry quality | glosses (definitions, not always short equivalents), part of speech, genders, examples for part of the entries, **inflection tables as `forms`** for German nouns, verbs and adjectives | short translation pairs; no inflection data, little grammar |
| Lemmatization | yes, from the data itself: every form in `forms` points back to its headword | no; would need another source |
| Size | English edition, all languages, is 24.8 GB of JSON (2.9 GB gzipped); the German edition 2.9 GB (296 MB gzipped). A German-only slice compiled to a compact index is expected in the tens of MB per pair. **Not measured**: the multi-gigabyte dumps were not downloaded | not stated on the download page; StarDict archives |

Not considered because their terms forbid redistribution: dict.cc, Linguee, PONS, Leo.

## Decision

**Wiktionary (kaikki.org extracts), CC BY-SA, as separate downloadable packs.** It is the only source that is clearly
allowed to be redistributed under conditions the project can meet, and the only one that carries the inflection data the
lemma lookup needs. FreeDict is rejected: the GPL, and no forms.

Before the first pack is built, one check is still open: read the notice that ships with the downloaded extract (and
`wiktextract`'s README) and confirm it says what Wiktionary says. That needs the large download and was not done here, so
**no license claim in this page replaces that check.**

## Plan once approved

1. `tools/` script (not part of the app): read the extract for German, keep headword, POS, glosses (shortened and
   ranked), up to two examples, gender/plural and verb forms, and the `forms` table; write one compact file per pair.
2. A `dawn-dict` crate with a read-only index (an FST or SQLite file), lookup by exact headword, by form (lemma) and by
   prefix (closest headwords for "No entry for ...").
3. Packs as catalog type `"dictionary"` per pair in the existing model store, SHA-256 verified like the language packs;
   hosted in a public Hugging Face repo `Second-Dawn-Lab/owll-dictionaries` with a model card (source, license, credit),
   catalog URLs pinned to the commit.
4. UI: the Dictionary tab of the word popup, a collapsible Dictionary panel in the Text tab and the editor, attribution in
   About and on the Language packs page.

Estimated as the next release's main piece of work: the extract tooling and the lemma tables are the larger part, the UI
reuses the word popup and the editor's panels.
