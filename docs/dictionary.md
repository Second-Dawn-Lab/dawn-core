# Offline dictionary (`dawn-dict`)

Status: **built.** The license of the kaikki.org data was confirmed: it is Wiktionary's (CC BY-SA, with GFDL); kaikki.org adds no
terms and only asks to cite wiktextract and link to kaikki.org. The pack is published as `Second-Dawn-Lab/owll-dictionaries`
and installs from the Language packs page, or the first time the Dictionary tab is opened.

## What a dictionary pack needs

Headword, part of speech, ranked translations, short examples, German gender/plural and verb forms, and **inflected
forms to base forms** ("erstaunt" to "erstaunen", "Häuser" to "Haus", "ging" to "gehen"). The last one decides the
choice: a plain word list cannot do it.

## Sources checked

| | Wiktionary through kaikki.org (wiktextract) | FreeDict (deu-eng, eng-deu) |
|---|---|---|
| Origin | Wiktionary, parsed into JSON lines by `wiktextract` | the `ding` database of TU Chemnitz (Frank Richter), converted to TEI |
| License | Wiktionary's text is **CC BY-SA** (4.0, and GFDL as a second option); kaikki.org adds no terms | **GPL-2+** according to Debian's copyright file for the package; the upstream license text inside the archives was not read |
| Duties | credit Wiktionary and its editors with a link; the pack stays CC BY-SA (share-alike). The app code is a separate work and stays MIT | GPL: the pack would have to be offered under the GPL with its source; mixing with an MIT app is arguable and not worth the risk |
| Coverage | the English edition has entries for words of almost every language with English glosses | 517,534 German to English and 460,315 English to German headwords; many more pairs, mostly small |
| Entry quality | glosses, part of speech, genders, examples, **inflection tables as `forms`** | short translation pairs; no inflection data, little grammar |
| Lemmatization | yes, from the data itself | no |

Not considered because their terms forbid redistribution: dict.cc, Linguee, PONS, Leo.

**Decision: Wiktionary (kaikki.org extracts), CC BY-SA, as separate downloadable packs.** FreeDict was rejected: the GPL, and no forms.

## What was built

* **Crate `dawn-dict`** (no dependencies besides serde): a read-only file per direction. Entries are JSON records; a sorted table of
  lowercase keys points at them, so a lookup is a binary search and nothing is parsed until an entry is asked for. A key is a
  headword or an **inflected form**; the file is read into memory (31 MB for German to English).
* **Lemma lookup** needs no rules: it uses Wiktionary's own data. A base form's `forms` table ("Haus": Hauses, Häuser; "gehen":
  ging, gegangen, geht, ...) and the entries that only say "inflection of X" both become keys that lead to X. A word that is a
  headword *and* a form ("erstaunt") returns both, the headword first. Table metadata, phrases ("ich gehe") and forms with
  spaces are not keys. Unknown words get the closest headwords (prefix, else one or two edits). Not covered: compound
  splitting (a German compound that is not an entry is not found) and forms Wiktionary has no table for.
* **German to English** comes from the German entries (English glosses, head line with gender/plural/verb forms, up to two
  examples). **English to German** comes from the `translations` of English entries (sense groups, with gender, without dialect
  and obsolete words) plus a reverse index built from the English glosses of the German entries ("house, building" gives
  "Haus n" under *house*); English inflected forms and "form of" entries come from the English extract. The English file alone only
  has German translations for about 5,100 words, which is why the reverse index is there.
* **Pack** `de-en.zip`: `info.json`, `NOTICE.txt` (credit), `de-en.dict`, `en-de.dict`. Catalog list `dictionaries`, own folder
  and list inside the model store (so removing a language never touches it), the same download and sha256 check as language
  packs, pinned to a commit of the Hugging Face repo.
* **Tools**: `dawn-dict-build to-english|from-english` (reads the kaikki.org JSONL, streamed from `curl` if wanted) and
  `tools/build-dictionary-pack.py` (zip + model card).

## Packs (v0.3.0)

Built with `dawn-dict-build pair <lang> <Language>` from the kaikki.org extracts of 2026-10; all 13 together are 119 MB to download. Thin means fewer than about 15,000 entries; **Beta** is set in the catalog for packs under 5,000 entries (none) and for Chinese.

| Language | Pack | to English (entries) | from English (entries) | Download | Quality |
|---|---|---|---|---|---|
| Arabic | `ar-en.zip` | 28,392 | 37,269 | 5.6 MB | fair; vowel marks are ignored when looking up |
| Bengali | `bn-en.zip` | 11,143 | 12,425 | 1.4 MB | thin |
| German | `de-en.zip` | 102,897 | 78,440 | 11.5 MB | good; gender, plural and verb forms |
| Spanish | `en-es.zip` | 124,377 | 92,281 | 15.1 MB | good |
| French | `en-fr.zip` | 103,247 | 85,971 | 9.9 MB | good |
| Hindi | `en-hi.zip` | 26,251 | 26,538 | 3.5 MB | fair |
| Italian | `en-it.zip` | 171,821 | 99,082 | 15.2 MB | large; many inflected forms are entries of their own |
| Japanese | `en-ja.zip` | 143,840 | 81,704 | 14.6 MB | large; verb forms work; text is not split into words, so only a single word clicked or selected is looked up |
| Punjabi | `en-pa.zip` | 8,046 | 10,597 | 1.2 MB | thin |
| Russian | `en-ru.zip` | 62,442 | 57,661 | 10.9 MB | good; stress marks are ignored when looking up |
| Turkish | `en-tr.zip` | 29,210 | 27,780 | 13.4 MB | fair; case and possessive forms only where Wiktionary lists them (evlerimizde leads to ev) |
| Urdu | `en-ur.zip` | 8,986 | 12,993 | 1.3 MB | thin |
| Chinese | `en-zh.zip` | 194,926 | 118,466 | 15.3 MB | **Beta**: headwords are mostly traditional characters, simplified-only words are often missing; no word splitting |

The smallest pack is 1.2 MB (Punjabi), the largest 15.3 MB (Chinese). Pairs without English are answered through English ("Meanings in English"); the pack of the other language is offered when it is missing.

## Rebuilding

```text
curl -O https://kaikki.org/dictionary/German/kaikki.org-dictionary-German.jsonl        (1.1 GB)
dawn-dict-build pair de German <folder>      (streams both extracts through curl; writes de-en.dict and en-de.dict)
python tools/build-dictionary-pack.py <folder> <out> --lang-a de --lang-b en     (one pack: <id>.zip and <id>.json)
python tools/dictionary-model-card.py <out> COMMIT --beta zh                      (README.md for the repo)
hf upload Second-Dawn-Lab/owll-dictionaries <out> . --include "*.zip" --include "README.md"
python tools/dictionary-model-card.py <out> <the commit the upload printed> --beta zh     (the catalog entries)
```

Then put the new commit hash, size and sha256 into `models/catalog.json` (`dictionaries`), and run `cargo test -p dawn-dict --test real_pack -- --ignored` with `DAWN_DICT_DIR` set.

## Credit (About, the Language packs page, the model card)

Wiktionary contributors (CC BY-SA 4.0), data extracted by wiktextract and published by kaikki.org. The packs stay CC BY-SA 4.0;
the app is MIT.

## Next

More pairs: the same tools work for any language the English edition has entries for (French, Spanish, ...): one more
`to-english` and `from-english` run and one more catalog entry per pair.
