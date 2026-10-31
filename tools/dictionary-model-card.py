"""Writes the Hugging Face model card (README.md) for the dictionary repo from the `<id>.json` summaries that
`build-dictionary-pack.py` left in a folder, and prints the `dictionaries` entries for `models/catalog.json`.

    python tools/dictionary-model-card.py <folder with the .json summaries and .zip files> <commit hash or "COMMIT"> [--beta zh,...]

Run it twice: once before the upload (commit "COMMIT": only the card is written), once after, with the real 40-digit commit hash
the upload printed, to get the catalog entries.
"""
import argparse
import glob
import json
import os

NAMES = {'ar': 'Arabic', 'bn': 'Bengali', 'de': 'German', 'en': 'English', 'es': 'Spanish', 'fr': 'French', 'hi': 'Hindi', 'it': 'Italian',
         'ja': 'Japanese', 'pa': 'Punjabi', 'ru': 'Russian', 'tr': 'Turkish', 'ur': 'Urdu', 'zh': 'Chinese'}

CREDIT = (
    'Contains data from Wiktionary (https://www.wiktionary.org), written by Wiktionary contributors and licensed under '
    'CC BY-SA 4.0 (and GFDL), extracted by wiktextract and published at https://kaikki.org. '
    'Changes: entries were selected, shortened and re-organized into a compact index; inflected forms were turned into lookup keys; '
    'the direction into the other language also lists the entries whose English glosses contain the word.'
)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('folder')
    ap.add_argument('commit')
    ap.add_argument('--beta', default='')
    args = ap.parse_args()
    beta = {b for b in args.beta.split(',') if b}
    packs = sorted((json.load(open(p, encoding='utf-8')) for p in glob.glob(os.path.join(args.folder, '*.json'))), key=lambda p: p['id'])
    rows = []
    for p in packs:
        other = [l for l in p['languages'] if l != 'en'][0]
        d = p['directions']
        entries = ', '.join(f"{k.replace('-', ' to ')} {v['entries']:,}".replace(' to ', ' → ') for k, v in sorted(d.items()))
        mark = ' (Beta)' if other in beta else ''
        rows.append(f"| `{p['zip']}` | {NAMES[other]}{mark} | {entries} | {p['sizeBytes'] / 1e6:.1f} MB | `{p['sha256']}` |")
    card = f"""---
license: cc-by-sa-4.0
language:
{chr(10).join('- ' + l for l in sorted({l for p in packs for l in p['languages']}))}
tags:
- dictionary
- wiktionary
- offline
---

# Owl-L dictionaries

Offline dictionary packs for [Owl-L](https://github.com/Second-Dawn-Lab) (the Second Dawn translator). The app downloads a pack
once, checks its SHA-256 and then looks words up on the computer without any network. One pack per language pair, both directions inside.

| Pack | Language with English | Entries | Download | SHA-256 |
|---|---|---|---|---|
{chr(10).join(rows)}

Each entry has the headword, part of speech, the head line with grammar (gender, plural, verb forms where Wiktionary has them), ranked
senses with translations or glosses, and short examples. Inflected forms are lookup keys that lead to their base form ("Häuser" to
"Haus", "ging" to "gehen"), taken from Wiktionary's own inflection tables. Pairs without English (German to French) are answered through
English by the app.

Quality varies with how complete the English edition of Wiktionary is for the language: large for French, Spanish, Italian, Russian, German,
Japanese and Chinese; small for Urdu, Punjabi, Bengali, Hindi, Turkish and Arabic. Chinese headwords are mostly traditional characters, so
simplified-only words are often missing (Beta).

## Source, license and credit

{CREDIT}

* **License of these packs: [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/).** If you reuse or adapt them, you must credit
  Wiktionary and its contributors, link to the license, say what you changed, and share your version under the same license.
* Wiktionary: <https://www.wiktionary.org> (entries and their authors are listed in each page's history).
* kaikki.org / wiktextract: <https://kaikki.org>, Tatu Ylonen, "Wiktextract: Wiktionary as Machine-Readable Structured Data", LREC 2022.
* The Owl-L application code is MIT licensed; these data packs are a separate work under CC BY-SA 4.0.

## How they are made

`dawn-dict-build pair <lang> <Language>` (in [dawn-core](https://github.com/Second-Dawn-Lab/dawn-core), `crates/dawn-dict`) streams the kaikki.org
extracts of the English edition of Wiktionary and writes the compact index; `tools/build-dictionary-pack.py` zips the files. Nothing else is added.
"""
    with open(os.path.join(args.folder, 'README.md'), 'w', encoding='utf-8') as f:
        f.write(card)
    if args.commit != 'COMMIT':
        entries = []
        for p in packs:
            other = [l for l in p['languages'] if l != 'en'][0]
            biggest = max(v['entries'] for v in p['directions'].values())
            entries.append({
                'id': p['id'],
                'languages': p['languages'],
                'urls': [f"https://huggingface.co/Second-Dawn-Lab/owll-dictionaries/resolve/{args.commit}/{p['zip']}"],
                'sizeBytes': p['sizeBytes'],
                'sha256': p['sha256'],
                'license': 'CC-BY-SA-4.0',
                'attribution': 'Wiktionary contributors (CC BY-SA 4.0), data extracted by wiktextract and published by kaikki.org',
                'sourceUrl': 'https://en.wiktionary.org',
                'beta': other in beta or biggest < 5000,
                'entries': biggest,
            })
        print(json.dumps(entries, indent=2, ensure_ascii=False))


if __name__ == '__main__':
    main()
