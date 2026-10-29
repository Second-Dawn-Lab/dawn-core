"""Helpers for reading and writing models/catalog.json (format version 2, see docs/models.md)."""

from __future__ import annotations

import json
from pathlib import Path


def migrate(catalog: dict) -> dict:
    """Turn a version 1 catalog (models inline in each language, with from/to) into version 2."""
    if catalog.get("version", 1) >= 2:
        catalog.setdefault("models", [])
        return catalog
    models: dict[str, dict] = {}
    languages = []
    for lang in catalog.get("languages", []):
        ids = []
        for m in lang.get("models", []):
            model = {k: v for k, v in m.items() if k not in ("from", "to")}
            model = {"id": m["id"], "sources": [m["from"]], "targets": [m["to"]], **{k: v for k, v in model.items() if k != "id"}}
            models.setdefault(m["id"], model)
            ids.append(m["id"])
        languages.append({**{k: v for k, v in lang.items() if k != "models"}, "models": ids})
    return {"version": 2, "hub": catalog.get("hub", "en"), "models": list(models.values()), "languages": languages}


def load(path: Path) -> dict:
    if path.exists():
        return migrate(json.loads(path.read_text(encoding="utf-8")))
    return {"version": 2, "hub": "en", "models": [], "languages": []}


def save(path: Path, catalog: dict) -> None:
    catalog["languages"] = sorted(catalog["languages"], key=lambda l: l["code"])
    catalog["models"] = sorted(catalog["models"], key=lambda m: m["id"])
    path.write_text(json.dumps(catalog, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def upsert_model(catalog: dict, model: dict) -> None:
    catalog["models"] = [m for m in catalog["models"] if m["id"] != model["id"]] + [model]


def upsert_language(catalog: dict, language: dict) -> None:
    catalog["languages"] = [l for l in catalog["languages"] if l["code"] != language["code"]] + [language]
