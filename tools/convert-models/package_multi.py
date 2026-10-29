#!/usr/bin/env python3
"""Package an already converted multilingual CTranslate2 model and add it to the catalog.

For models that serve several languages (for example the Opus-MT tc-bible `-inc` model, which
translates English/German/French/Spanish/Portuguese into Urdu, Hindi, Punjabi and Bengali).
Convert first with `ct2-transformers-converter --quantization int8 --copy_files source.spm target.spm`,
then:

    python package_multi.py --dir out/_research/bible-inc-ct2-int8 \\
        --id opus-mt-tc-bible-big-inc-ct2-int8 \\
        --source-repo Helsinki-NLP/opus-mt-tc-bible-big-deu_eng_fra_por_spa-inc --license Apache-2.0 \\
        --sources en de fr es pt --targets ur hi pa bn \\
        --token ur=">>urd<<" --token hi=">>hin<<" --token pa=">>pan<<" --token bn=">>ben<<" \\
        --catalog ../../models/catalog.json

Languages are added to the catalog separately with --language code:Name:NativeName[:beta]
(repeatable); each lists this model, so several languages can share it.
"""

from __future__ import annotations

import argparse
import json
import zipfile
from pathlib import Path

import catalog_tools
from convert import ATTRIBUTION, sha256_of as sha256_file


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--dir", type=Path, required=True, help="converted CTranslate2 model folder")
    p.add_argument("--id", required=True, help="model id, e.g. opus-mt-tc-bible-big-inc-ct2-int8")
    p.add_argument("--source-repo", required=True, help="Hugging Face repo it was converted from")
    p.add_argument("--license", required=True)
    p.add_argument("--sources", nargs="+", required=True)
    p.add_argument("--targets", nargs="+", required=True)
    p.add_argument("--token", action="append", default=[], help="LANG=TOKEN, the token selecting a target language")
    p.add_argument("--prefix", help="single token for single-target models")
    p.add_argument("--language", action="append", default=[], help="CODE:Name:NativeName[:beta] to list this model")
    p.add_argument("--out", type=Path, default=Path(__file__).parent / "out")
    p.add_argument("--catalog", type=Path, required=True)
    args = p.parse_args()

    for needed in ("model.bin", "source.spm", "target.spm", "shared_vocabulary.json"):
        if not (args.dir / needed).exists():
            raise SystemExit(f"{args.dir} is missing {needed}")

    tokens = dict(t.split("=", 1) for t in args.token)
    vocab = set(json.loads((args.dir / "shared_vocabulary.json").read_text(encoding="utf-8")))
    for lang, token in tokens.items():
        if token not in vocab:
            raise SystemExit(f"token {token} for {lang} is not in the model vocabulary")

    manifest = {
        "id": args.id,
        "sources": args.sources,
        "targets": args.targets,
        "targetTokens": tokens,
        "source": f"https://huggingface.co/{args.source_repo}",
        "license": args.license,
        "attribution": ATTRIBUTION,
        "conversion": {"tool": "ct2-transformers-converter", "quantization": "int8"},
    }
    (args.dir / "manifest.json").write_text(json.dumps(manifest, indent=2, ensure_ascii=False), encoding="utf-8")

    args.out.mkdir(parents=True, exist_ok=True)
    archive = args.out / f"{args.id}.zip"
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as zf:
        for path in sorted(args.dir.rglob("*")):
            if path.is_file():
                zf.write(path, path.relative_to(args.dir).as_posix())

    model = {
        "id": args.id,
        "sources": args.sources,
        "targets": args.targets,
        "url": archive.resolve().as_uri(),
        "sizeBytes": archive.stat().st_size,
        "sha256": sha256_file(archive),
        "license": args.license,
        "attribution": ATTRIBUTION,
    }
    if tokens:
        model["targetTokens"] = tokens
    if args.prefix:
        model["prefix"] = args.prefix

    catalog = catalog_tools.load(args.catalog)
    catalog_tools.upsert_model(catalog, model)
    for spec in args.language:
        parts = spec.split(":")
        code, name, native = parts[0], parts[1], parts[2]
        existing = next((l for l in catalog["languages"] if l["code"] == code), None)
        ids = list(existing["models"]) if existing else []
        if args.id not in ids:
            ids.append(args.id)
        language = {"code": code, "name": name, "nativeName": native}
        if "beta" in parts[3:] or (existing or {}).get("beta"):
            language["beta"] = True
        language["models"] = ids
        catalog_tools.upsert_language(catalog, language)
    catalog_tools.save(args.catalog, catalog)
    print(f"{archive.name}: {archive.stat().st_size / 1e6:.1f} MB, sha256 {model['sha256'][:12]}..., catalog updated")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
