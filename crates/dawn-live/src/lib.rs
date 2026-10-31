//! The logic behind live captions, without the screen, the OCR engine or the windows.
//!
//! A loop (in the app) grabs a small region of the screen again and again. For every grab:
//!
//! 1. [`change`]: did the pixels change since the last one? No change, no OCR. A region that stays completely black
//!    is reported (protected video cannot be captured).
//! 2. [`stable`]: text that builds up word by word is not translated until two reads in a row agree (or a short
//!    timeout passes), so the caption bar does not flicker.
//! 3. [`lines`]: of the stable text only what is new is translated; scrolling captions and repeated lines are handled.
//! 4. [`rate`]: how long to wait for the next grab (calmer when nothing changes, on battery, or when OCR is slow).
//! 5. [`place`]: where the caption bar goes, on any monitor, kept on screen.

pub mod change;
pub mod lines;
pub mod pipeline;
pub mod place;
pub mod rate;
pub mod stable;

pub use change::{is_black, signature, BlackEvent, BlackWatch, ChangeDetector, Signature};
pub use lines::{group_sentences, split_lines, Delta, LineTracker};
pub use pipeline::{Pipeline, Step};
pub use place::{physical, place_bar, place_popup, Rect};
pub use rate::{interval_ms, Speed};
pub use stable::{nearly_same, similarity, Stabilizer, Verdict};
