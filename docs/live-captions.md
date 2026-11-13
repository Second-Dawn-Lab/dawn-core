# Live captions (`dawn-live`, `dawn-capture::region`)

Translate text that keeps changing on screen: video subtitles, live captions in a call, a game's dialogue box, a slideshow.
Start with **Ctrl+Shift+L** (Settings, Keyboard shortcuts; conflict check like the other shortcuts), the main menu, the tray
menu or the **Live captions** button in the Images tab. The area-capture overlay appears; draw the region. A caption bar
opens below it (kept on screen, flipped above when there is no room) and keeps showing the translation. Stop with the bar's
close button, the shortcut again, or Esc in the bar.

## How a look at the region works

```text
grab region  ->  changed?  --no-->  same text as before (nothing is read)
                    | yes
                  read (Windows OCR, one scale, one line per screen line)
                    |
        two reads in a row agree (or 1.5 s)?  --no-->  wait   (captions that build up word by word)
                    | yes
        lines that are new?  (scrolling, growing last line, repeats)  -->  translate only those, once
```

* **Grab**: `dawn_capture::Grabber` copies only the region (`xcap`'s `capture_region`, a GDI `BitBlt` of that rectangle). No new
  dependency.
* **Change detection** (`change.rs`): the frame is reduced to 48x12 blocks of average brightness (25 samples per block); at least two
  blocks must differ by 24 levels. Compression noise changes single blocks by a few levels and is ignored; a new word changes several
  by tens. The reference only moves on a change, so a slow fade adds up. No change means **no OCR**: the previous text is fed
  to the stabilizer again, which is how a caption becomes stable without a second read.
* **Stable text** (`stable.rs`): a read is passed on when the next one is the same text (up to one letter in eight may differ: OCR slips
  such as "m" read as "rn"), or after 1.5 s of a text that never settles (moving video behind the captions). Empty reads twice in a row mean
  the captions went away.
* **New lines** (`lines.rs`): the lines are screen lines. The longest overlap between the end of the old lines and the start of the new ones is
  scrolling, so only the rest is new. A last line that got longer replaces its earlier form. The same words again after the captions
  cleared are new; while they stay on screen they are not. New lines that do not end a sentence are translated together with the next
  one (`group_sentences`).
* **Rate** (`rate.rs`): Calm 800 ms, Normal 500 ms, Fast 250 ms between looks; x1.5 after 5 s without change, x2 after 15 s, x3 after a
  minute, x1.6 on battery; never less than three times the last read's duration (so the processor share stays bounded), never more than 3 s.
* **Black region** (`change.rs`): a region whose every sampled pixel is black for 4 s, with no text ever read in it, is reported:
  "completely black ... protected video". Black between subtitles of a video that did have text is silent. Nothing tries to get around it.
* **Placement** (`place.rs`): centered below the region, above it when there is no room below, inside its bottom edge when there is room on
  neither side, always inside the monitor of the region's center (monitors left of or above the primary one have negative coordinates;
  sizes are converted per monitor scale).

## The caption bar

The bar shows the **conversation of the session**: every line (up to 2,000 in memory) with its time, in a list that follows the newest line while the reader
is at the bottom, stops following when the reader scrolls up, and shows "Jump to latest (N new)" (`scrollModel.ts`). The bar can be resized taller; its size is kept
in the settings. When the session ends the bar stays open and asks whether to save the conversation to the History (Save / Don't save, "Remember my choice" stores
Always or Never); a saved session is one History entry whose lines are kept in a file of their own (`live-sessions/<id>.json`), opened from the History tab.
Language lists and menus of the bar (and of the quick popup) open in a small window of their own (`picker.rs`), because a drop-down inside a small window is cut off.

A transparent window with a rounded surface, always on top, not in the taskbar, draggable and resizable, in the app's theme (light and dark),
with 1 to 3 lines (setting), text size 16 to 48 px, background opacity 30 to 100 %, an optional small original line, RTL output through
`dir="auto"`. On hover: language pickers, pause/resume, history (about 50 lines, copy, export to a .txt file), change region, lock,
settings, close. **Lock** makes the bar click-through (`set_ignore_cursor_events`); the tray entry "Unlock the caption bar" or the
shortcut unlocks it (the next press stops). Nothing is written to the History unless the user answers Save (or chose Always).

The bar and the overlays call `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`, so the bar never appears in what the loop grabs and
can sit over the region. This needs Windows 10 version 2004 or later; on older systems the call fails and the bar should be kept
outside the region (the default placement does that).

## Why a GDI region copy and not Windows Graphics Capture

Measured on a 3440x1440 monitor (`cargo run --release -p dawn-capture --example grab_bench`, CPU time of the process, 1000x150 region):

| Method | CPU per grab | Wall per grab |
|---|---|---|
| whole monitor (`capture_image`), then crop | 30 ms | 67 to 79 ms |
| region only (`capture_region`) | **0.9 ms** | 16.7 ms (waits for the desktop compositor, not CPU) |

Windows Graphics Capture delivers whole frames from the GPU; taking a small region still means copying the whole frame (or a GPU
crop plus read-back), needs a Direct3D device and a frame pool, and keeps the compositor busy at the capture frame rate. For a
region read twice a second the region copy is far cheaper. WGC would be the choice for capturing a *window* that is covered or
minimized, which this feature does not do. Known limits of the GDI copy: protected (DRM) video comes out black; exclusive-fullscreen
games are not captured.

## Measured (this machine: 3440x1440, Windows OCR, German model, Normal speed)

`cargo test -p owll live_sequence -- --ignored --nocapture` plays 21 generated pictures (subtitles appearing word by word, then
scrolling over two lines, with gaps): 7 of the 21 looks changed and were read (**86 ms per read**), 4 lines were translated, each
once (**131 ms per line**), and every expected line was found. From the picture that completed a caption to its translation:
**about 600 to 690 ms** at Normal speed (one more look 500 ms later confirms the text, then read and translation).

CPU while captions run (**estimated from the measured numbers above, not from a long run with a real video**; the Task Manager check is in
`owll/docs/manual-tests-v0.3.md`): a look at an unchanged region costs about 1 ms (the grab) plus the signature; a read about 90 ms; a
translation about 130 ms per line. At Normal speed with a caption every 3 seconds that is two looks per second at 1 ms each plus one
read and one translation per caption: well under 5 % of one core, which is under 1 % of a 6-core laptop. Continuously changing video behind
the captions makes every look a read: at 500 ms intervals that is about 90 ms per 500 ms, about 18 % of one core (3 % of 6 cores); the
interval stretches to three times the read duration when OCR is slower.

## Not covered

* Speech-to-text (translating audio) is out of scope; it would be a separate engine behind the same bar.
* A region that was moved on screen (the video window moved) has to be drawn again ("Change region").
* Exclusive-fullscreen games and DRM video are not capturable; the black-region message says so for the latter.

## Meeting captions (Teams, Zoom, Meet) (`dawn-live::meeting`)

The caption area of a meeting app is not subtitles: an avatar with initials, a speaker name, then one or two lines of text that keep growing while the person speaks, and for
every speaker change another block below. Waiting for two equal reads (the plain-subtitle rule) makes the translation lag by a second or more, and the name would be translated as a word.
The pipeline therefore has a second mode, `MeetingMode` (setting `liveMeeting`: automatic, always, never). In automatic mode the first speaker name that is read switches the region to meeting captions.

* **Layout** (`parse_meeting`): the recognized lines are split into one block per speaker. Initials are dropped (on a line of their own, in front of the name, or in front of the first text line; only when they are the initials of that speaker, so "IT will be ready" stays). A speaker line is two to four capitalized words (particles such as "van" and hyphens allowed), no end punctuation, no digits, or a label such as "Speaker 1" / "Sprecher 2"; words that start sentences ("Good morning", "Thank you") are not names. The name is a label: it is never part of the translated text.
* **Following** (`MeetingTracker`): every utterance has an id. Each read updates the text of the last block (the one being spoken); a changed text is reported at once, so the translation of the growing line is shown right away and replaced as it grows. When the caption box scrolls (its first words leave) the utterance keeps all its words, found by the overlap of the old end and the new start; a cut-off last word is completed; an OCR slip in a word or the name does not start a new utterance. Blocks above the live one, and utterances of another speaker, are finished; so is an utterance that did not change for 1.8 s. Two empty reads in a row end everything, so the same words later are new.
* **Translation and polish** (`live.rs`): the whole utterance is translated at each change (the engine's sentence cache makes earlier sentences free, so only the growing one costs time). A finished utterance is polished in the background when "Polish live captions too" is on and the model is loaded and answers within 8 s; the polished text replaces the fast one (same id) and the first polish only loads the model.
* **The bar**: lines are keyed by utterance id, show the speaker as a label, grow in place, and the transcript, the copy and the saved session contain "Name: text".

Measured delay from a change of the caption to its translation (the loop of `run` played on a virtual clock with the real OCR and the German model; looks every 250 / 500 / 800 ms, one change every 450 ms in the synthetic test, one every 600 ms in the local recording test; caption translations with the cache warm):

| Speed | synthetic captions (made-up name, a word every 450 ms) | a real recording (70 frames, local test) |
|---|---|---|
| Fast | mean 0.31 s, worst 0.52 s | mean 0.28 s, 90% 0.39 s, worst 0.45 s |
| Normal | mean 0.54 s, worst 1.14 s | mean 0.45 s, 90% 0.64 s, worst 0.77 s |
| Calm | mean 0.48 s, worst 0.99 s | mean 0.41 s, 90% 0.69 s, worst 0.79 s |

Reading a frame took 88 ms and translating a growing line 60 to 100 ms on the test machine. In the real recording the speaker name was read in every frame and 17 utterances were found, all with a speaker label. The synthetic test is `cargo test -p owll meeting_delay -- --ignored --nocapture`. The real-recording test (`src-tauri/src/live_local_test.rs`) is **local only**: the file is git-ignored, it is only built when it exists, it reads frames from a folder named by `OWLL_LIVE_FRAMES` (frames are saved from the recording with a local script, outside every repository) and it prints numbers only, never names or sentences. Nothing of the recording, its frames, its OCR text or transcripts is in a repository, a fixture, a document or a commit message.

Not covered: meeting captions are already speech recognition and contain its errors; translating the audio directly (a local Whisper model) would be a separate, later feature and is out of scope here.

### Second round (meeting captions)

* **The main window comes back.** When the main window was open and stepped aside for the region selection, closing the caption bar (its button, Esc, the shortcut, or the answer to "Save?") brings it back to the front (`live::close_bar`, `should_restore_main`). A main window that was already closed to the tray stays there. Changing the region does not forget that the window stepped aside.
* **Speech-recognition errors.** A finished utterance is polished from its source text with the "intended meaning" prompt (see `quality-mode.md`): the model is told that the text may contain missing, wrong or run-together words and translates what was meant, without adding information. The fast translation shown while the speaker talks is a plain machine translation of what was read; a sentence it could not translate cleanly (a loop) is shown up to the loop, or as the source, and the polish does not start from it.
