# Area selection and text recognition (OCR)

Press the capture shortcut (default **Ctrl+Shift+O**, Settings, Keyboard shortcuts), the capture button in the Text or Images tab, or copy an image and press Ctrl+C+C: Owl-L reads the text in it and shows the translation.

## Flow

1. **Freeze.** `dawn-capture` screenshots every monitor first (`xcap`). The overlay windows show those frozen screenshots, so the overlay is never in the capture and a playing video is frozen at the moment of the key press. Screenshots are kept in memory only (never written to disk, never in the history, which stores text only) and dropped when the selection ends.
2. **Select.** One borderless always-on-top overlay per monitor, placed over the monitor exactly. Drag a rectangle; Esc or right-click cancels, and a drag under 8 pixels cancels. The size of the selection is shown while dragging. The selection stays on the monitor where the drag began.
3. **Map.** The page reports the rectangle in CSS pixels. `dawn_capture::map::to_image_rect` scales it by *screenshot size / viewport size*, per monitor, so mixed DPI (100 % next to 150 %) and rounding need no special cases. Unit-tested without needing a second monitor.
4. **Recognize.** Small images are enlarged (2x, 3x for very flat selections) before recognition, which helps both engines. Lines are joined into sentences (`dawn_ocr::join`): lines that continue a sentence are joined with a space, a hyphenated word at a line end is stitched together, paragraph gaps and list items stay as line breaks.
5. **Show.** The quick popup opens below the selection (flipping above when there is no room) with the translation, "Show recognized text" (editable; editing translates again), Copy, "Open in Owl-L", pin and close. No Replace. "No text found in the selected area" offers "Try again".

Capture is an explicit action, so the **block list does not apply** to it (it applies to Ctrl+C+C only).

## Engines

| Engine | Status |
| --- | --- |
| Windows OCR (`Windows.Media.Ocr`) | Implemented. No download. Works for the languages installed as "Optical character recognition" in Windows (Settings, Time & language, Language & region, language options). |
| RapidOCR / PaddleOCR (ONNX Runtime) | **Not implemented, waiting for a decision** (see below). |

The capture languages (Settings, Files, up to three) default to English plus the active languages. Each is tried with Windows OCR; with several, the result with the most letters wins. A language Windows lacks is shown as "Not available" with a button that opens Windows' language settings. If none of the capture languages is available, the popup says which one to install.

On the development machine Windows OCR offers **de-DE and en-GB only**. French, Spanish, Italian, Arabic, Urdu and others are missing there (Windows has no Urdu OCR at all, as far as we know; Arabic needs its language pack).

### RapidOCR: size

Measured with a minimal program (release, LTO):

| | executable |
| --- | --- |
| no ONNX Runtime | 0.11 MB |
| `ort` with `load-dynamic` (loads `onnxruntime.dll` at runtime) | 0.16 MB (+0.05 MB) |
| `ort` with the runtime linked in | 18.97 MB (+18.9 MB) |

Linking ONNX Runtime in adds about 19 MB to Owl-L's executable, which is too much for a lightweight app. Proposal: use `load-dynamic` and ship `onnxruntime.dll` (about 11 to 13 MB, not measured here) together with the models as a downloadable "OCR pack" through the existing model store (`"type": "ocr"` in `models/catalog.json`), so users who never need it pay nothing. Models per script are roughly 5 to 15 MB (detector, recognizer; sizes from memory, not measured).

Script coverage of PaddleOCR (from its documentation, **not verified here**): Chinese and English, Latin languages, Japanese, Korean, Arabic-script languages (Arabic, Persian, Urdu, Uyghur), Cyrillic, Devanagari (Hindi), Tamil, Telugu, Kannada. Not covered: Bengali and Punjabi (Gurmukhi) as far as we know.

## Limits

- Exclusive-fullscreen games may not be capturable by the screen-capture APIs; borderless-window games usually are.
- The first capture creates the overlay windows, which takes longer than later ones (they are kept hidden).
- Mixed-DPI and multi-monitor handling is covered by unit tests of the coordinate mapping, but was not tried on a second monitor.
