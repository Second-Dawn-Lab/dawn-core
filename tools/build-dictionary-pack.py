"""Packs the dictionary files of one language pair into the zip the app downloads, and writes the model card.

    python tools/build-dictionary-pack.py <folder with de-en.dict and en-de.dict> <out folder> [--lang-a de --lang-b en]

The dictionary files come from `dawn-dict-build` (see docs/dictionary.md). Output: `<out>/en-de.zip` and
`<out>/<id>.json` (sizes, counts, sha256 for `models/catalog.json` and the model card; see `dictionary-model-card.py`).
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
    summary = {
        'id': pack_id,
        'languages': pair,
        'zip': os.path.basename(zip_path),
        'sizeBytes': size,
        'sha256': digest,
        'built': info['built'],
        'directions': {n.replace('.dict', ''): {'entries': counts[n][0], 'keys': counts[n][1], 'bytes': os.path.getsize(os.path.join(args.folder, n))} for n in files},
    }
    with open(os.path.join(args.out, f'{pack_id}.json'), 'w', encoding='utf-8') as f:
        json.dump(summary, f, indent=2)
    print(json.dumps(summary))


if __name__ == '__main__':
    main()
