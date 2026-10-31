"""Packs the dictionary files of one language pair into the zip the app downloads, and writes the model card.

    python tools/build-dictionary-pack.py <folder with de-en.dict and en-de.dict> <out folder> [--lang-a de --lang-b en]

The dictionary files come from `dawn-dict-build` (see docs/dictionary.md). Output: `<out>/en-de.zip` and
`<out>/README.md` (the Hugging Face model card). Prints the size and sha256 for `models/catalog.json`.
"""
import argparse
import hashlib
import json
import os
import zipfile
from datetime import date

CREDIT = (
    'Contains data from Wiktionary (https://www.wiktionary.org), written by Wiktionary contributors and licensed under '
    'CC BY-SA 4.0 (and GFDL), extracted by wiktextract and published at https://kaikki.org. '
    'Changes: entries were selected, shortened and re-organized into a compact index; inflected forms were turned into lookup keys; '
    'the English-to-German direction also lists the German entries whose English glosses contain the word.'
)


def sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for block in iter(lambda: f.read(1 << 20), b''):
            h.update(block)
    return h.hexdigest()


def entries_of(path):
    # header: magic(8) version(4) src(4) dst(4) n_entries(4) n_keys(4)
    with open(path, 'rb') as f:
        head = f.read(28)
    return int.from_bytes(head[20:24], 'little'), int.from_bytes(head[24:28], 'little')


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('folder')
    ap.add_argument('out')
    ap.add_argument('--lang-a', default='de')
    ap.add_argument('--lang-b', default='en')
    args = ap.parse_args()
    pair = sorted([args.lang_a, args.lang_b])
    pack_id = f'{pair[0]}-{pair[1]}'
    files = [f'{args.lang_a}-{args.lang_b}.dict', f'{args.lang_b}-{args.lang_a}.dict']
    counts = {name: entries_of(os.path.join(args.folder, name)) for name in files}
    info = {
        'id': pack_id,
        'languages': pair,
        'directions': [n.replace('.dict', '') for n in files],
        'format': 'dawn-dict 1',
        'license': 'CC-BY-SA-4.0',
        'source': 'Wiktionary (English edition) through kaikki.org / wiktextract',
        'built': date.today().isoformat(),
        'entries': {n.replace('.dict', ''): counts[n][0] for n in files},
    }
    os.makedirs(args.out, exist_ok=True)
    zip_path = os.path.join(args.out, f'{pack_id}.zip')
    with zipfile.ZipFile(zip_path, 'w', zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        z.writestr('info.json', json.dumps(info, indent=2))
        z.writestr('NOTICE.txt', CREDIT + '\n\nThis dictionary pack is licensed under CC BY-SA 4.0: https://creativecommons.org/licenses/by-sa/4.0/\n')
        for name in files:
            z.write(os.path.join(args.folder, name), name)
    size = os.path.getsize(zip_path)
    digest = sha256(zip_path)
    card = f"""---
license: cc-by-sa-4.0
language:
- {pair[0]}
- {pair[1]}
tags:
- dictionary
- wiktionary
- offline
---

# Owl-L dictionaries

Offline dictionary packs for [Owl-L](https://github.com/Second-Dawn-Lab) (the Second Dawn translator). The app downloads a pack
once, checks its SHA-256 and then looks words up on the computer without any network.

## `{pack_id}.zip`

| | |
|---|---|
| Directions | {info['directions'][0]} ({counts[files[0]][0]:,} entries), {info['directions'][1]} ({counts[files[1]][0]:,} entries) |
| Size | {size / 1e6:.1f} MB zip (the files inside are about {sum(os.path.getsize(os.path.join(args.folder, n)) for n in files) / 1e6:.0f} MB) |
| SHA-256 | `{digest}` |
| Built | {info['built']} |

Each entry has the headword, part of speech, the head line with grammar (gender, plural, verb forms), ranked senses with
translations or glosses, and short examples where Wiktionary has them. Inflected forms ("Häuser", "ging") are lookup keys
that lead to their base form ("Haus", "gehen"), taken from Wiktionary's own inflection tables.

## Source, license and credit

{CREDIT}

* **License of this pack: [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/).** If you reuse or adapt it, you must credit
  Wiktionary and its contributors, link to the license, say what you changed, and share your version under the same license.
* Wiktionary: <https://www.wiktionary.org> (entries and their authors are listed in each page's history).
* kaikki.org / wiktextract: <https://kaikki.org>, Tatu Ylonen, "Wiktextract: Wiktionary as Machine-Readable Structured Data", LREC 2022.
* The Owl-L application code is MIT licensed; this data pack is a separate work under CC BY-SA 4.0.

## How it is made

`dawn-dict-build` (in [dawn-core](https://github.com/Second-Dawn-Lab/dawn-core), `crates/dawn-dict`) reads the kaikki.org extracts of the
English edition of Wiktionary and writes the compact index; `tools/build-dictionary-pack.py` zips the files. Nothing else is added.
"""
    with open(os.path.join(args.out, 'README.md'), 'w', encoding='utf-8') as f:
        f.write(card)
    print(json.dumps({'id': pack_id, 'zip': zip_path, 'sizeBytes': size, 'sha256': digest}, indent=2))


if __name__ == '__main__':
    main()
