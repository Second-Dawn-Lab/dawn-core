//! The logic behind live captions, without the screen, the OCR engine or the windows.
//!
//! A loop (in the app) grabs a small region of the screen again and again. For every grab:
//!
//! 1. [`change`]: did the pixels change since the last one? No change, no OCR. A region that stays completely black
//!    is reported (protected video cannot be captured).
//! 2. [`stable`]: text that builds up word by word is not translated until two reads in a row agree (or a short
//!    timeout passes), so the caption bar does not flicker.
//! 3. [`track`] and [`lines`]: captions are recognized by their words, so a line that scrolled up is the same line and only
//!    what is new or changed is translated; wrapped sentences stay one caption.
//! 4. [`rate`]: how long to wait for the next grab (calmer when nothing changes, on battery, or when OCR is slow).
//! 4b. [`meeting`]: Teams, Zoom and Meet captions (a speaker name above growing text) are followed word by word instead,
//!     one utterance per speaker block.
//! 5. [`place`]: where the caption bar goes, on any monitor, kept on screen.

pub mod change;
pub mod lines;
pub mod meeting;
pub mod pipeline;
pub mod place;
pub mod rate;
pub mod stable;
pub mod track;

pub use change::{is_black, signature, BlackEvent, BlackWatch, ChangeDetector, Signature};
pub use lines::{split_lines, LineTracker};
pub use meeting::{parse_meeting, parse_meeting_text, Block, MeetingTracker};
pub use pipeline::{MeetingMode, Pipeline, Step};
pub use place::{physical, place_bar, place_popup, Rect};
pub use rate::{interval_ms, Speed};
pub use stable::{nearly_same, similarity, Stabilizer, Verdict};
pub use track::CaptionUpdate;
