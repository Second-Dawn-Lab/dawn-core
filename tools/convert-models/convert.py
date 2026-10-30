#!/usr/bin/env python3
"""Convert Helsinki-NLP Opus-MT models to CTranslate2 int8 and package them for Second Dawn.

For every language code X this builds two models (en->X and X->en), archives each as a
zip with a manifest.json, and prints catalog entries ready for models/catalog.json.
See README.md.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen

HUB = "en"
ATTRIBUTION = "Helsinki-NLP / OPUS-MT"
# Opus-MT sometimes uses its own code for a language (e.g. Japanese is "jap" in en->ja).
HF_OVERRIDES = {
    ("en", "ja"): "Helsinki-NLP/opus-mt-en-jap",
    # There is no plain opus-mt-en-tr; the transformer-big "tc-big" model is the Turkish one.
    # (Fallback if it ever disappears: Helsinki-NLP/opus-mt-en-trk with the >>tur<< token.)
    ("en", "tr"): "Helsinki-NLP/opus-mt-tc-big-en-tr",
}

# Languages whose models are known to translate poorly; the catalog marks them as beta.
BETA_LANGUAGES = {"ur", "ja"}

NAMES = {
    "de": ("German", "Deutsch"),
    "ur": ("Urdu", "اردو"),
    "fr": ("French", "Français"),
    "es": ("Spanish", "Español"),
    "it": ("Italian", "Italiano"),
    "tr": ("Turkish", "Türkçe"),
    "ar": ("Arabic", "العربية"),
    "ru": ("Russian", "Русский"),
    "zh": ("Chinese", "中文"),
    "ja": ("Japanese", "日本語"),
}


def hf_id(src: str, dst: str) -> str:
    return HF_OVERRIDES.get((src, dst), f"Helsinki-NLP/opus-mt-{src}-{dst}")


# Multi-target Opus-MT models (e.g. en->zh) need a language token in front of every source
# sentence. These are the preferred tokens per target language; the script only uses one that
# exists in the model's vocabulary, and warns otherwise instead of guessing.
PREFIX_CHOICES = {
    "zh": [">>cmn_Hans<<", ">>cmn<<"],
    # opus-mt-en-ar has no >>arb<<; >>ara<< is its generic Arabic token.
    "ar": [">>ara<<"],
    "ja": [">>jpn<<", ">>jpn_Hira<<"],
}


def find_prefix(hf_dir: Path, src: str, dst: str, model_id: str) -> str | None:
    vocab_file = hf_dir / "vocab.json"
    if not vocab_file.exists():
        return None
    tokens = [t for t in json.loads(vocab_file.read_text(encoding="utf-8")) if t.startswith(">>") and t.endswith("<<")]
    if not tokens or src != HUB:
        return None
    for choice in PREFIX_CHOICES.get(dst, []):
        if choice in tokens:
            return choice
    print(f"  WARNING [{model_id}]: model needs a target-language token; none of the preferred ones exist. "
          f"Available: {', '.join(tokens[:20])}{' ...' if len(tokens) > 20 else ''}")
    return None


def hf_info(repo: str) -> dict | None:
    """Model metadata from the Hugging Face API, or None if the repo does not exist."""
    req = Request(f"https://huggingface.co/api/models/{repo}", headers={"User-Agent": "dawn-convert"})
    try:
        with urlopen(req, timeout=30) as resp:
            return json.load(resp)
    except HTTPError as err:
        if err.code in (401, 404):
            return None
        raise


def license_of(info: dict) -> str:
    card = info.get("cardData") or {}
    lic = card.get("license")
    if lic:
        return {"cc-by-4.0": "CC-BY-4.0", "apache-2.0": "Apache-2.0"}.get(lic, lic)
    for tag in info.get("tags", []):
        if tag.startswith("license:"):
            return tag.split(":", 1)[1]
    return "unknown"


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def convert_one(src: str, dst: str, info: dict, out: Path, work: Path) -> dict:
    """Download, convert and package one direction. Returns its catalog model entry."""
    from huggingface_hub import snapshot_download

    repo = hf_id(src, dst)
    model_id = f"opus-mt-{src}-{dst}-ct2-int8"
    model_dir = out / model_id
    archive = out / f"{model_id}.zip"

    if not archive.exists():
        print(f"  [{model_id}] downloading {repo}")
        hf_dir = Path(
            snapshot_download(
                repo,
                local_dir=str(work / repo.split("/")[1]),
                allow_patterns=["*.json", "*.spm", "*.bin", "*.safetensors", "*.txt", "*.model", "*.vocab"],
            )
        )
        if model_dir.exists():
            shutil.rmtree(model_dir)
        print(f"  [{model_id}] converting to CTranslate2 int8")
        subprocess.run(
            [
                sys.executable, "-m", "ctranslate2.converters.transformers",
                "--model", str(hf_dir),
                "--output_dir", str(model_dir),
                "--quantization", "int8",
                "--copy_files", "source.spm", "target.spm",
                "--force",
            ],
            check=True,
        )
        for needed in ("model.bin", "source.spm", "target.spm"):
            if not (model_dir / needed).exists():
                raise RuntimeError(f"{model_id}: converted model is missing {needed}")

        import ctranslate2

        prefix = find_prefix(hf_dir, src, dst, model_id)
        manifest = {
            "prefix": prefix,
            "id": model_id,
            "direction": {"from": src, "to": dst},
            "source": f"https://huggingface.co/{repo}",
            "license": license_of(info),
            "attribution": ATTRIBUTION,
            "conversion": {"tool": "ct2-transformers-converter", "quantization": "int8", "ctranslate2": ctranslate2.__version__},
        }
        (model_dir / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")

        print(f"  [{model_id}] packaging {archive.name}")
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as zf:
            for path in sorted(model_dir.rglob("*")):
                if path.is_file():
                    zf.write(path, path.relative_to(model_dir).as_posix())
        shutil.rmtree(model_dir)

    with zipfile.ZipFile(archive) as zf:
        prefix = json.loads(zf.read("manifest.json")).get("prefix")
    manifest_license = license_of(info)
    entry = {
        "id": model_id,
        "from": src,
        "to": dst,
        "urls": [archive.resolve().as_uri()],
        "sizeBytes": archive.stat().st_size,
        "sha256": sha256_of(archive),
        "license": manifest_license,
        "attribution": ATTRIBUTION,
    }
    if prefix:
        entry["prefix"] = prefix
    return entry


def merge_into_catalog(catalog_path: Path, languages: list[dict]) -> None:
    """Add per-language entries (models inline, with from/to) to a version 2 catalog."""
    import catalog_tools

    catalog = catalog_tools.load(catalog_path)
    for lang in languages:
        legacy = {"version": 1, "languages": [lang]}
        migrated = catalog_tools.migrate(legacy)
        for model in migrated["models"]:
            catalog_tools.upsert_model(catalog, model)
        catalog_tools.upsert_language(catalog, migrated["languages"][0])
    catalog_tools.save(catalog_path, catalog)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("languages", nargs="+", help="language codes, e.g. de ur fr")
    parser.add_argument("--out", type=Path, default=Path(__file__).parent / "out", help="output folder (git-ignored)")
    parser.add_argument("--check-only", action="store_true", help="only report which directions exist on Hugging Face")
    parser.add_argument("--update-catalog", type=Path, help="merge the entries into this catalog.json")
    args = parser.parse_args()

    out: Path = args.out
    work = out / "_hf"
    out.mkdir(parents=True, exist_ok=True)

    entries: list[dict] = []
    missing: list[str] = []
    for code in args.languages:
        if code == HUB:
            continue
        forward, backward = (HUB, code), (code, HUB)
        infos = {d: hf_info(hf_id(*d)) for d in (forward, backward)}
        gone = [hf_id(*d) for d, i in infos.items() if i is None]
        for repo in gone:
            print(f"MISSING: {repo}")
        if gone:
            missing.append(f"{code}: " + ", ".join(gone))
            continue
        print(f"{code}: both directions exist")
        if args.check_only:
            continue
        name, native = NAMES.get(code, (code, code))
        models = [convert_one(*d, infos[d], out, work) for d in (forward, backward)]
        entry = {"code": code, "name": name, "nativeName": native}
        if code in BETA_LANGUAGES:
            entry["beta"] = True
        entry["models"] = models
        entries.append(entry)

    if entries:
        snippet = out / ("catalog.entries." + "-".join(args.languages) + ".json")
        snippet.write_text(json.dumps(entries, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        print(f"\nCatalog entries written to {snippet}")
        if args.update_catalog:
            merge_into_catalog(args.update_catalog, entries)
            print(f"Merged into {args.update_catalog}")
        for lang in entries:
            total = sum(m["sizeBytes"] for m in lang["models"])
            print(f"  {lang['code']}: {total / 1e6:.1f} MB")

    if missing:
        print("\nLanguages with a missing direction (skipped, no substitute guessed):")
        for line in missing:
            print(f"  {line}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
