//! Second Dawn global hotkeys.
//!
//! Detects shortcuts such as Ctrl+C pressed twice (`Ctrl+C C`) without taking keys away from
//! other programs. The pieces:
//!
//! - [`shortcut`]: parsing, display and conflict detection for shortcuts.
//! - [`detector`]: the pure double-press state machine (no OS calls, fully unit-tested).
//! - [`engine`]: one detector per configured shortcut.
//! - `windows`: the keyboard hook, foreground app and clipboard counter (Windows only for now).
//!   macOS and Linux add their own module with the same four functions and `Hotkeys` type.

pub mod detector;
pub mod engine;
pub mod shortcut;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{
    clipboard_sequence, cursor_position, focus_window, foreground_process_name, foreground_window, left_button_down, send_paste,
    window_exists, Hotkeys,
};

#[cfg(not(windows))]
mod unsupported {
    use crate::shortcut::Shortcut;
    use std::io;

    /// Placeholder until macOS and Linux get an implementation.
    pub struct Hotkeys;

    impl Hotkeys {
        pub fn start(_shortcuts: Vec<(String, Shortcut)>, _on_event: impl Fn(&str, crate::detector::Outcome) + Send + 'static) -> io::Result<Self> {
            Err(io::Error::new(io::ErrorKind::Unsupported, "global shortcuts are only implemented on Windows so far"))
        }
        pub fn set_shortcuts(&self, _shortcuts: Vec<(String, Shortcut)>) {}
    }
    pub fn foreground_process_name() -> Option<String> {
        None
    }
    pub fn clipboard_sequence() -> u32 {
        0
    }
    pub fn cursor_position() -> Option<(i32, i32)> {
        None
    }
    pub fn foreground_window() -> Option<isize> {
        None
    }
    pub fn window_exists(_hwnd: isize) -> bool {
        false
    }
    pub fn focus_window(_hwnd: isize) -> bool {
        false
    }
    pub fn send_paste() {}
    pub fn left_button_down() -> bool {
        false
    }
}
#[cfg(not(windows))]
pub use unsupported::{
    clipboard_sequence, cursor_position, focus_window, foreground_process_name, foreground_window, left_button_down, send_paste,
    window_exists, Hotkeys,
};

/// Whether `process` (a file name such as "Code.exe") is on the block list. Names compare
/// case-insensitively; an entry may omit ".exe".
pub fn is_blocked(block_list: &[String], process: &str) -> bool {
    let norm = |s: &str| {
        let s = s.trim().to_lowercase();
        s.strip_suffix(".exe").map(str::to_string).unwrap_or(s)
    };
    let p = norm(process);
    !p.is_empty() && block_list.iter().any(|b| norm(b) == p)
}

#[cfg(test)]
mod tests {
    use super::is_blocked;

    #[test]
    fn block_list_matches_process_names_loosely() {
        let list = vec!["KeePass.exe".to_string(), " code ".to_string()];
        assert!(is_blocked(&list, "keepass.exe"));
        assert!(is_blocked(&list, "Code.exe"));
        assert!(is_blocked(&list, "CODE"));
        assert!(!is_blocked(&list, "codex.exe"));
        assert!(!is_blocked(&list, "chrome.exe"));
        assert!(!is_blocked(&list, ""));
        assert!(!is_blocked(&[], "chrome.exe"));
    }
}
