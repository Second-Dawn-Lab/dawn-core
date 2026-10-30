#!/usr/bin/env python
"""Writes the Hugging Face model card (README.md) for the language-pack repo from models/catalog.json.

    python make_model_card.py [output-path]      # default: out/README.md

The card lists every pack with its source model, license and attribution, so it can never drift from the catalog.
"""
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
CATALOG = HERE.parent.parent / "models" / "catalog.json"
REPO = "Second-Dawn-Lab/owll-language-packs"


def source_model(pack_id: str) -> str:
    # "opus-mt-de-en-ct2-int8" -> "opus-mt-de-en"
    return pack_id.removesuffix("-ct2-int8")


def mb(n: int) -> str:
    return f"{n / 1_000_000:.0f} MB"


def main() -> None:
    catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
    rows = []
    for m in sorted(catalog["models"], key=lambda x: x["id"]):
        src = source_model(m["id"])
        name = m["id"] + ".zip"
        langs = f"{', '.join(m['sources'])} → {', '.join(m['targets'])}"
        rows.append(
            f"| `{name}` | {langs} | [Helsinki-NLP/{src}](https://huggingface.co/Helsinki-NLP/{src}) "
            f"| {m['license']} | {m['attribution']} | {mb(m['sizeBytes'])} | `{m['sha256']}` |"
        )
    table = "\n".join(rows)
    total = sum(m["sizeBytes"] for m in catalog["models"])

    card = f"""---
license: other
license_name: cc-by-4.0-or-apache-2.0
library_name: ctranslate2
tags:
  - translation
  - ctranslate2
  - opus-mt
  - offline
---

# Owl-L language packs

Offline translation models for [Owl-L](https://github.com/Second-Dawn-Lab/owll), a free, offline, private translator
for the desktop, built on the shared [dawn-core](https://github.com/Second-Dawn-Lab/dawn-core) libraries.

Each file is one **language pack**: a zip archive with a [CTranslate2](https://github.com/OpenNMT/CTranslate2) model
(`model.bin` with int8 weights, `config.json`, `shared_vocabulary.json`), the two SentencePiece models
(`source.spm`, `target.spm`) and a `manifest.json` describing the pack. Owl-L downloads the packs a user asks for,
verifies the sha256 listed below (and in `models/catalog.json` of dawn-core) and unpacks them into its model store.
Nothing here is meant to be used by hand, but the archives are plain zips and work with any CTranslate2 setup.

## How Owl-L uses them

- English is the hub: packs go to or from English; other pairs are translated through English.
- Everything runs on the user's machine. The only network access is downloading a pack from this repository.
- Download URLs in the catalog are pinned to a commit of this repository, so a URL always returns exactly the file
  whose sha256 is listed.

## Packs ({len(catalog["models"])} files, {total / 1_000_000_000:.1f} GB)

| File | Languages | Source model | License | Attribution | Size | sha256 |
|---|---|---|---|---|---|---|
{table}

## Conversion, license and attribution

The weights come from the [OPUS-MT](https://github.com/Helsinki-NLP/Opus-MT) models by the Helsinki-NLP group
(University of Helsinki), released under **CC-BY-4.0** or **Apache-2.0** as shown per pack above. They were
**converted to the CTranslate2 format with int8 quantization**; no other change was made to the models.
**The license of each model is unchanged**, and the attribution "Helsinki-NLP / OPUS-MT" applies to every file.
If you reuse a pack, keep that attribution and, for CC-BY-4.0 models, indicate that the weights were converted.

Please cite the OPUS-MT papers listed on the source model cards.

Owl-L is an independent project of the Second Dawn contributors (MIT license for the code).
The converter is in [dawn-core/tools/convert-models](https://github.com/Second-Dawn-Lab/dawn-core/tree/main/tools/convert-models).
"""
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else HERE / "out" / "README.md"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(card, encoding="utf-8", newline="\n")
    print("wrote", out)


if __name__ == "__main__":
    main()
