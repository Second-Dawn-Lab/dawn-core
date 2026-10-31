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

A transparent window with a rounded surface, always on top, not in the taskbar, draggable and resizable, in the app's theme (light and dark),
with 1 to 3 lines (setting), text size 16 to 48 px, background opacity 30 to 100 %, an optional small original line, RTL output through
`dir="auto"`. On hover: language pickers, pause/resume, history (about 50 lines, copy, export to a .txt file), change region, lock,
settings, close. **Lock** makes the bar click-through (`set_ignore_cursor_events`); the tray entry "Unlock the caption bar" or the
shortcut unlocks it (the next press stops). Lines are only kept in the History when "Save live caption transcripts" is on (off by default).

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
