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

## Namespaces and Excel text cells

Elements are matched by **namespace URI and local name, never by prefix**: files written by the .NET OpenXML SDK or
ClosedXML use `x:` (or `d:` for DrawingML), other writers `w:`, `a:` or none, and all are valid. The transitional
(2006) and the ISO strict URIs are both recognized, and an element named `t` from another namespace (math, a custom
part) is ignored. The tests have prefixed, default-namespace and strict fixtures for Word, PowerPoint and Excel.

Excel stores text in three ways, and all three are handled: shared strings (`xl/sharedStrings.xml`), inline strings
(`t="inlineStr"`) and **`t="str"` cells whose text is in `<v>`** (what ClosedXML writes, with an empty shared string
table). A `t="str"` cell that has an `<f>` formula is a formula *result* and is left alone. Numbers and dates are
not text. (In v0.2.0 only the first two were handled, so a workbook written the ClosedXML way came back unchanged.)

## Several documents at once

The Files tab takes any number of files of any mix of types (drop or "Choose files", "Add more documents" later). The
list has one row per file with its detected language (changeable per file), status (waiting, translating with
progress, done, error) and open / show in folder / remove. One target language and one glossary apply to all. Files
run one after another in the background; "Cancel all" stops the running one (nothing is written for it) and clears the
queue. The queue is a pure state machine (`batchQueue.ts`) with tests for the order, a failing file not stopping the
rest, cancelling and removing. Each finished file adds one History entry.

## Keep as PDF: evaluation (not built)

The question to answer is whether a *translated PDF with the original layout* can reach acceptable quality now.
Findings, with the permissively licensed options:

| | PDFium (`pdfium-render`, MIT or Apache-2.0; PDFium BSD-3/Apache-2.0) | Content streams with `lopdf` (MIT) |
|---|---|---|
| Size | `pdfium.dll` for Windows x64 is about 11 MB installed (3.9 MB download); the crate adds little | no engine; fonts only |
| What it can do | read text objects with boxes, remove them, add text objects with an embedded TrueType font | edit `Tj`/`TJ` operators |
| Problem 1: fonts | text in a PDF uses subset fonts with custom encodings; the original glyphs cannot be reused for new text, so every replaced word needs a font that covers the target script, embedded in the file. Latin and Cyrillic need one font (about 0.5 MB), Arabic/Urdu, Devanagari and CJK each their own (CJK 8 to 16 MB) | the same, and the font objects must be written by hand |
| Problem 2: shaping | neither PDFium nor `lopdf` shapes text: Arabic and Urdu letters must be joined and reordered, Devanagari needs conjuncts. That needs a shaping library (`rustybuzz`, MIT) and drawing positioned glyphs, not strings | the same |
| Problem 3: layout | a "text block" does not exist in a PDF. Lines, columns and table cells are loose text runs; grouping them into paragraphs, translating a paragraph, and refitting it into the original box (shrinking the font) needs a layout analysis. Multi-column pages, tables, text on paths, rotated text and headers that overlap images break it | the same |
| Problem 4: right to left | the box must be mirrored and the text aligned right | the same |

My estimate: Latin and Cyrillic targets for single-column text PDFs could be made to look acceptable with PDFium and
about one font, but Arabic/Urdu, Devanagari and CJK need a shaping and glyph-drawing step on top, and the
layout analysis is the largest part for every language. That is a separate piece of work (weeks, not part of
a patch release), and a result that is wrong on many real PDFs would be worse than the clear Word conversion. So
"Keep as PDF" is **not built**: the choice setting and its rules exist (`pdfChoice.ts`, `pdfOutput` and `pdfAsk`),
and today every PDF is converted to a Word document without asking. If you want it, the recommended route is PDFium
as a downloadable pack (like OCR packs would be) plus one Latin/Cyrillic font, for single-column text PDFs only, with a
clear "layout may differ" note, and the other scripts to follow with shaping.

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

## Translate and edit (`Session`)

`Session::open(path)` reads a file as the editor needs it: every text segment in reading order, **with its place** and its
formatting, and (for Office files) the segments that are left as they were.

* **Places** (`Header`): Word has body, header, footer, footnotes, endnotes, comments with the paragraph number in that
  part; PowerPoint has the slide's position in the *presentation order* (read from `presentation.xml`, not the file name)
  and notes pages with the slide they belong to; Excel has the sheet name (from `workbook.xml`) and the cell of the
  shared string (the first cell that shows it); PDF has the page; text files the paragraph. The UI turns this into
  "Slide 4 · Notes", "Sheet Sales · B5", "Page 2".
* **Formatting**: a segment with several formats keeps its `{1}..{/1}` markers; `style_from_key` reads bold, italic,
  underline and links from the run properties of each stretch (`<w:b/>`, `<x:i/>`, `b="1"` all work, a size or
  complex-script bold does not count). The editor shows bold as bold and hides the markers.
* `Session::translate` is the same batching as `translate_document` (repeated texts once). `Session::save(translations, out)`
  uses the existing writers, so everything not translated stays byte for byte as in the original; the written file is
  opened again with the crate's own loader before it replaces anything (`.part` file, then rename). A damaged marker in an
  edited text falls back to the proportional mapping above, so no text is lost.
* `Fingerprint` (size, date, hash of the document's texts) decides whether a draft still fits its file: *same* (also for a
  copy with another date), *changed* or *missing*.
* Markdown is refused for editing: its code and link targets are tokens an editor must not show.
* A 300-page document (12,000 paragraphs) opens in about 1 s and saves in about 1.2 s in a debug build.
