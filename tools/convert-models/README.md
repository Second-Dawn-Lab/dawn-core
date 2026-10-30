# convert-models

Converts [Helsinki-NLP Opus-MT](https://huggingface.co/Helsinki-NLP) models to CTranslate2 int8 and packages them for Second Dawn language packs. See [`docs/models.md`](../../docs/models.md) for the design.

For every language code `X` it builds two models, `en->X` and `X->en`, as `opus-mt-<from>-<to>-ct2-int8.zip` (with `model.bin`, the SentencePiece files and a `manifest.json`).

## Setup

Needs Python 3.10+. The first install downloads PyTorch (CPU is enough).

```bash
cd tools/convert-models
py -3.12 -m venv .venv
.venv/Scripts/python -m pip install -r requirements.txt --extra-index-url https://download.pytorch.org/whl/cpu
```

## Run

```bash
# Which directions exist on Hugging Face? (no downloads)
.venv/Scripts/python convert.py de ur fr es it tr ar ru zh ja --check-only

# Convert German and Urdu, and merge the entries into the catalog
.venv/Scripts/python convert.py de ur --update-catalog ../../models/catalog.json
```

- Output goes to `out/` (git-ignored): the zips, `catalog.entries.<languages>.json` for pasting, and a Hugging Face download cache in `out/_hf/`.
- The converter writes `urls` as a `file://` url to the zip in `out/`, so you can test the whole download flow locally. Before publishing, upload the zips (see "Hosting the packs" in [docs/models.md](../../docs/models.md)) and replace the urls with the commit-pinned Hugging Face urls.
- `make_model_card.py` writes the model card (`out/README.md`) for the Hugging Face repo from `models/catalog.json`.
- A direction that does not exist on Hugging Face is reported and the language is skipped. Nothing is substituted.
- Multi-target models (such as `en->zh`) need a language token; the script records it as `prefix` if the model's vocabulary contains one of its preferred tokens, and warns otherwise.
- Existing zips are reused; delete one to convert it again.

Model files must never be committed.
