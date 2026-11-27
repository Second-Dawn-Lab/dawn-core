//! Captions that scroll up, as a meeting app or rolling subtitles show them: a few dialogs in the area, the newest one
//! growing word by word at the bottom, the older ones moving up and leaving at the top. Generated from made-up names and
//! text and played through the whole pipeline at each speed, with the mistakes a screen reader makes.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use dawn_live::{CaptionUpdate, MeetingMode, Pipeline};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 120;
/// A new word appears this often.
const WORD_EVERY_MS: u64 = 300;
/// The pause between two dialogs.
const BETWEEN_MS: u64 = 700;
const WORDS_PER_LINE: usize = 6;
/// Dialogs the area has room for.
const VISIBLE: usize = 3;
/// The looks of Fast, Normal and Calm.
const SPEEDS: [u64; 3] = [250, 500, 800];

/// A picture that differs whenever the text does.
fn picture(text: &str) -> Vec<u8> {
    let mut hash = DefaultHasher::new();
    text.hash(&mut hash);
    let hash = hash.finish();
    let mut pixels = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let block = (y / 20) * 12 + x / 40;
            let level: u8 = if !text.is_empty() && (hash >> (block % 64)) & 1 == 1 { 230 } else { 30 };
            let i = ((y * WIDTH + x) * 4) as usize;
            pixels[i..i + 4].copy_from_slice(&[level, level, level, 255]);
        }
    }
    pixels
}

#[derive(Clone, Copy)]
struct Dialog {
    speaker: &'static str,
    text: &'static str,
    /// The caption app corrects its words after it has moved up: (as first shown, corrected).
    correction: Option<(&'static str, &'static str)>,
}

const fn says(speaker: &'static str, text: &'static str) -> Dialog {
    Dialog { speaker, text, correction: None }
}

/// Dialogs with wrapped sentences, two speakers taking turns (one twice in a row), short answers that repeat, and a late
/// correction.
fn meeting() -> Vec<Dialog> {
    vec![
        says("Mara Winter", "Good morning, today we plan the spring fair for the library."),
        says("Tobias Brenner", "I have booked the large room on the ground floor already."),
        says("Mara Winter", "Yes."),
        Dialog { speaker: "Tobias Brenner", text: "The bakery on the corner will bring fresh bread and two cakes.", correction: Some(("cakes", "pies")) },
        says("Tobias Brenner", "We still need volunteers for the book stall in the afternoon."),
        says("Mara Winter", "Yes."),
        says("Mara Winter", "I will ask the school whether the choir can sing at three."),
        says("Tobias Brenner", "Then I print the posters on Friday and hang them up on Monday."),
        says("Mara Winter", "Thank you, that was everything for this week."),
    ]
}

/// The words of a dialog as shown after `shown` of them, with the late correction applied or not yet.
fn wording(d: &Dialog, corrected: bool) -> Vec<String> {
    let text = match d.correction {
        Some((first, then)) if !corrected => d.text.replace(then, first),
        Some((first, then)) => d.text.replace(first, then),
        None => d.text.to_string(),
    };
    text.split(' ').map(str::to_string).collect()
}

/// An OCR slip: one letter of a long word is misread.
fn slip(line: &str) -> String {
    let mut words: Vec<String> = line.split(' ').map(str::to_string).collect();
    if let Some(word) = words.iter_mut().find(|w| w.chars().count() >= 8 && w.contains('e')) {
        *word = word.replacen('e', "c", 1);
    }
    words.join(" ")
}

#[derive(Clone, Copy, PartialEq)]
enum Shape {
    /// An avatar with initials and the speaker's name above the text of each dialog.
    Meeting,
    /// Only the text, line by line.
    Plain,
}

/// The screen at time `now`.
fn screen(dialogs: &[Dialog], shape: Shape, now: u64) -> String {
    let mut begins = 0;
    let mut shown: Vec<(usize, usize)> = Vec::new(); // (dialog, words)
    for (i, d) in dialogs.iter().enumerate() {
        let count = wording(d, false).len();
        if now >= begins {
            shown.push((i, (((now - begins) / WORD_EVERY_MS) as usize + 1).min(count)));
        }
        begins += count as u64 * WORD_EVERY_MS + BETWEEN_MS;
    }
    if now > begins + 6000 {
        return String::new();
    }
    let first = shown.len().saturating_sub(VISIBLE);
    let mut blocks: Vec<Vec<String>> = Vec::new();
    for (n, &(i, count)) in shown.iter().enumerate().skip(first) {
        let d = &dialogs[i];
        // The correction arrives just after the next dialog has started (the dialog has moved up).
        let corrected = shown.len() > i + 1;
        let words = &wording(d, corrected)[..count];
        let mut lines: Vec<String> = words.chunks(WORDS_PER_LINE).map(|c| c.join(" ")).collect();
        // The oldest dialog in a full area is cut off: its name and first line are above the edge.
        let cut = shown.len() > VISIBLE && n == first && lines.len() > 1;
        if cut {
            lines.remove(0);
        } else if shape == Shape::Meeting {
            let initials: String = d.speaker.split(' ').filter_map(|w| w.chars().next()).collect();
            lines.insert(0, d.speaker.to_string());
            lines.insert(0, initials);
        }
        blocks.push(lines);
    }
    let mut lines: Vec<String> = blocks.concat();
    // Every fifth moment the reader gets one letter wrong somewhere.
    if (now / 400) % 5 == 4 {
        let at = (now / 400) as usize % lines.len().max(1);
        if let Some(line) = lines.get_mut(at) {
            *line = slip(line);
        }
    }
    lines.join("\n")
}

struct Played {
    /// The conversation as the bar keeps it: (id, speaker, text), a new id at the end.
    shown: Vec<(u64, Option<String>, String)>,
    /// Every text a caption was given, in order, by position in `shown`.
    wordings: Vec<Vec<String>>,
}

fn play(dialogs: &[Dialog], shape: Shape, every_ms: u64) -> Played {
    let mut pipeline = Pipeline::new();
    pipeline.set_meeting_mode(if shape == Shape::Meeting { MeetingMode::Auto } else { MeetingMode::Off });
    let total: u64 = dialogs.iter().map(|d| wording(d, false).len() as u64 * WORD_EVERY_MS + BETWEEN_MS).sum::<u64>() + 9000;
    let mut played = Played { shown: Vec::new(), wordings: Vec::new() };
    let mut now = 0;
    while now < total {
        let text = screen(dialogs, shape, now);
        let step = pipeline.step(now, &picture(&text), WIDTH, HEIGHT, &mut || Some(text.clone()));
        for CaptionUpdate { id, speaker, text, .. } in step.updates {
            let at = played.shown.iter().position(|(known, ..)| *known == id).unwrap_or_else(|| {
                played.shown.push((id, speaker.clone(), String::new()));
                played.wordings.push(Vec::new());
                played.shown.len() - 1
            });
            if played.shown[at].2 != text {
                played.shown[at].2 = text.clone();
                played.wordings[at].push(text);
            }
        }
        now += every_ms;
    }
    played
}

fn plain_words(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric() || *c == ' ').collect::<String>().to_lowercase()
}

fn like(a: &str, b: &str) -> bool {
    dawn_live::similarity(&plain_words(a), &plain_words(b)) >= 0.93
}

#[test]
fn a_scrolling_meeting_gives_each_dialog_once_in_the_order_spoken() {
    let dialogs = meeting();
    for every_ms in SPEEDS {
        let played = play(&dialogs, Shape::Meeting, every_ms);
        let list: Vec<&str> = played.shown.iter().map(|(_, _, t)| t.as_str()).collect();
        // No duplicates, nothing lost, nothing mixed: one line per dialog, in order, with its speaker and its words.
        assert_eq!(played.shown.len(), dialogs.len(), "every {every_ms} ms: {list:#?}");
        for ((_, speaker, text), d) in played.shown.iter().zip(&dialogs) {
            assert_eq!(speaker.as_deref(), Some(d.speaker), "every {every_ms} ms: {list:#?}");
            assert!(like(text, &wording(d, true).join(" ")), "every {every_ms} ms: {text:?} is not {:?}", d.text);
        }
        assert!(played.shown.windows(2).all(|w| w[0].0 < w[1].0), "ids grow in the order spoken");
    }
}

#[test]
fn a_dialog_that_moved_up_is_not_translated_again_but_a_late_correction_is_taken() {
    let dialogs = meeting();
    for every_ms in SPEEDS {
        let played = play(&dialogs, Shape::Meeting, every_ms);
        for (wordings, d) in played.wordings.iter().zip(&dialogs) {
            let whole = wording(d, false).join(" ");
            // Once the whole dialog was read, its words only change for the correction: moving up and OCR slips change nothing.
            let complete = wordings.iter().position(|t| like(t, &whole)).unwrap_or_else(|| panic!("every {every_ms} ms: {whole:?} never read whole: {wordings:?}"));
            let after: Vec<&String> = wordings[complete + 1..].iter().filter(|t| plain_words(t).split(' ').count() >= whole.split(' ').count()).collect();
            let allowed = usize::from(d.correction.is_some());
            // The newest words can be read once more with a slip while the dialog is still the growing one.
            assert!(after.len() <= allowed + 1, "every {every_ms} ms: {:?} was given again: {wordings:?}", d.text);
            assert!(wordings.windows(2).all(|w| w[0] != w[1]));
            if let Some((_, then)) = d.correction {
                assert!(wordings.last().unwrap().contains(then), "every {every_ms} ms: the correction arrived in place: {wordings:?}");
            }
        }
    }
}

#[test]
fn scrolling_plain_captions_give_each_sentence_once_and_in_order() {
    let dialogs: Vec<Dialog> = meeting().into_iter().filter(|d| d.text.len() > 5).collect();
    for every_ms in SPEEDS {
        let played = play(&dialogs, Shape::Plain, every_ms);
        let list: Vec<&str> = played.shown.iter().map(|(_, _, t)| t.as_str()).collect();
        assert_eq!(played.shown.len(), dialogs.len(), "every {every_ms} ms: {list:#?}");
        for ((_, speaker, text), d) in played.shown.iter().zip(&dialogs) {
            assert_eq!(*speaker, None);
            assert!(like(text, &wording(d, true).join(" ")), "every {every_ms} ms: {text:?} is not {:?}", d.text);
        }
    }
}
