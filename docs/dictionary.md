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

## Pack sizes

| Pair | Direction | Entries | Keys | File |
|---|---|---|---|---|
| German and English | German to English | 102,897 | 482,296 | 30.6 MB |
| | English to German | 78,440 | 151,614 | 14.6 MB |
| | both, `de-en.zip` | | | 45.2 MB raw, **11.5 MB download** |

## Rebuilding

```text
curl -O https://kaikki.org/dictionary/German/kaikki.org-dictionary-German.jsonl        (1.1 GB)
dawn-dict-build to-english de kaikki.org-dictionary-German.jsonl de-en.dict
curl -sL https://kaikki.org/dictionary/English/kaikki.org-dictionary-English.jsonl |      (3.4 GB, streamed)
  dawn-dict-build from-english de kaikki.org-dictionary-German.jsonl - en-de.dict
python tools/build-dictionary-pack.py <folder with both .dict> <out>
hf upload Second-Dawn-Lab/owll-dictionaries <out> . --include "de-en.zip" --include "README.md"
```

Then put the new commit hash, size and sha256 into `models/catalog.json` (`dictionaries`).

## Credit (About, the Language packs page, the model card)

Wiktionary contributors (CC BY-SA 4.0), data extracted by wiktextract and published by kaikki.org. The packs stay CC BY-SA 4.0;
the app is MIT.

## Next

More pairs: the same tools work for any language the English edition has entries for (French, Spanish, ...): one more
`to-english` and `from-english` run and one more catalog entry per pair.
