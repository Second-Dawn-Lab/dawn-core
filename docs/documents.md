# Document translation (`dawn-docs`)

The crate reads a file into text *units* (a paragraph, a table cell, a slide title), a `Translator` turns the units
into translations, and the format writes them back. It knows nothing about engines: Owl-L plugs in `dawn-translate`,
tests plug in a fake. Everything runs locally; documents, glossary and history never leave the computer.

## What each format does

| Format | Output | Kept | Translated | Not translated / lost |
|---|---|---|---|---|
| `.docx` | `.docx` | styles, numbering, tables, images, theme, settings, relationships (byte for byte); formatting of runs | body, tables, text boxes, headers, footers, footnotes, endnotes, comments | charts, SmartArt, math, field codes, hidden text, text in embedded objects. Bold or italic *inside* a sentence: see below |
| `.pptx` | `.pptx` | layouts, masters, media, animations (byte for byte) | slide text incl. tables and grouped shapes, speaker notes | charts, SmartArt, text inside images |
| `.xlsx` | `.xlsx` | formulas, cached values, numbers, dates, styles, sheet names, defined names | text cells (shared strings), inline strings, cell comments, text boxes in drawings | formulas that compare with fixed text may need adjusting (listed in the report); table header cells (Excel ties them to the table definition) |
| `.txt` | `.txt` | line breaks, blank lines, indentation, line ending style (LF/CRLF), final newline, BOM or UTF-16 | paragraphs | numbers-only blocks |
| `.md` | `.md` | fenced and indented code, inline code, front matter, HTML, comments, reference definitions, link and image targets, emphasis marks, list and heading markers, table separators | text, table cells | |
| `.pdf` | `.docx` | the text of each page in reading order, paragraphs re-formed from lines, a page break per page | all text | **the layout** (columns, fonts, positions, images). Output is a Word document. Scanned PDFs (no text layer) are refused; OCR for PDFs comes later |

Always skipped inside text: web and e-mail addresses (kept as written), numbers-only text, formulas, identifiers
(`snake_case`, paths).

## Inline formatting

A paragraph is a row of runs (`This is a ` plain, `very important` bold, ` sentence.`). Translating run by run would cut
sentences in pieces, so a paragraph is translated as one text with a marker pair around each differently formatted
stretch:

```text
{1}Please read the {/1}{2}user manual{/2}{3} carefully.{/3}
{1}Bitte lesen Sie die {/1}{2}Bedienungsanleitung{/2}{3} sorgfältig.{/3}
```

The text between a marker pair goes back into the runs with that formatting. Markers were chosen by testing the real
model: `{1}..{/1}` and `[1]..[/1]` survived 5 of 5 test sentences, `<b1>` 5/5, `<1>` 4/5, `[[1]]` and `*..*` 3/5.

If the model damages a marker (missing, repeated, out of order), the markers are stripped and the plain translation
is cut at word boundaries in proportion to the original run lengths, so no text is lost or misplaced, but the
formatting lands only approximately. What is lost, honestly: when the model moves a word across a marker ("lazy dog" became
"den ... Hund"), the bold covers the wrong word. Paragraphs with a single format (the common case), headings, table
cells and slide titles are exact. A tab, line break or picture in the middle of a paragraph splits it into separate
units.

## Safety

* The input file is never modified. The output is written to `name.part` and renamed when it is complete, so a
  cancelled or failed job leaves no partial file; the default name is `name (DE).ext`, never an existing file.
* Office files are edited as text splices: the byte ranges of the `<w:t>` contents are found once and only those
  ranges change. Every other byte of the XML part stays; every other zip entry is copied as stored (not even
  recompressed). `xml:space="preserve"` is added where translated text begins or ends with a space.
* Each rewritten part is parsed again; if it is not well formed it is kept as it was and listed in the report.
* A password-protected file (an OLE container) reports "encrypted", a file that is not a zip or lacks its main part
  "corrupt", a PDF without text "scanned". Files over 100 MB are refused (they are read into memory).
* A job is cancellable between batches of 16 units; repeated texts are translated once.

## Adding a format

Implement `load(path) -> Loaded { units, parts, skipped, writer }` and a `Writer` that writes the output from
`translations[i]` (one per unit), then add the extension to `Kind`. The shared `ooxml` module already handles any
Office XML part given a `Dialect` (element names for paragraph, run, text, properties, breaks).

## Tests

Fixtures are generated in the tests (zips built in code, a PDF built with `lopdf`), no copyrighted documents. They check
that untouched parts are byte identical, formulas, numbers, dates and sheet names stay, Markdown code is not touched,
markers round trip, damaged markers fall back, malformed parts are skipped. `tests/with_models.rs` (ignored) translates
generated `.docx`, `.pptx` and `.xlsx` with the real German model and checks the results.
