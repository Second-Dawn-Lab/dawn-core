//! Runs one `Detector` per configured shortcut. Platform independent; the platform layer only
//! feeds it key events.

use crate::detector::{Detector, KeyEvent, Outcome, DEFAULT_WINDOW_MS};
use crate::shortcut::Shortcut;

pub struct Engine {
    detectors: Vec<(String, Detector)>,
    window_ms: u64,
}

impl Engine {
    pub fn new(window_ms: u64) -> Self {
        Self { detectors: Vec::new(), window_ms }
    }

    /// Replace the shortcuts. Half-finished presses are forgotten.
    pub fn set_shortcuts(&mut self, shortcuts: Vec<(String, Shortcut)>) {
        self.detectors = shortcuts.into_iter().map(|(name, s)| (name, Detector::new(s, self.window_ms))).collect();
    }

    /// The shortcuts that did something on this event: fired, or saw the first press of a double.
    pub fn on_event(&mut self, ev: KeyEvent) -> Vec<(String, Outcome)> {
        self.detectors
            .iter_mut()
            .filter_map(|(name, d)| match d.on_event(ev) {
                Outcome::Nothing => None,
                outcome => Some((name.clone(), outcome)),
            })
            .collect()
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new(DEFAULT_WINDOW_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(vk: u32, down: bool, time_ms: u64) -> KeyEvent {
        KeyEvent { vk, down, time_ms }
    }

    #[test]
    fn each_shortcut_fires_on_its_own() {
        let mut e = Engine::default();
        e.set_shortcuts(vec![("translate".into(), "Ctrl+C C".parse().unwrap()), ("capture".into(), "Ctrl+F8".parse().unwrap())]);
        assert!(e.on_event(ev(0xA2, true, 0)).is_empty());
        assert_eq!(e.on_event(ev(0x43, true, 10)), vec![("translate".to_string(), Outcome::FirstPress)]);
        assert!(e.on_event(ev(0x43, false, 30)).is_empty());
        assert_eq!(e.on_event(ev(0x43, true, 100)), vec![("translate".to_string(), Outcome::Fired)]);
        assert_eq!(e.on_event(ev(0x77, true, 200)), vec![("capture".to_string(), Outcome::Fired)]);
    }

    #[test]
    fn reconfiguring_forgets_a_half_finished_press() {
        let mut e = Engine::default();
        e.set_shortcuts(vec![("translate".into(), Shortcut::ctrl_c_c())]);
        e.on_event(ev(0xA2, true, 0));
        e.on_event(ev(0x43, true, 10));
        e.set_shortcuts(vec![("translate".into(), Shortcut::ctrl_c_c())]);
        e.on_event(ev(0xA2, true, 20));
        let out = e.on_event(ev(0x43, true, 50));
        assert_eq!(out, vec![("translate".to_string(), Outcome::FirstPress)], "the press before the change no longer counts");
    }
}
