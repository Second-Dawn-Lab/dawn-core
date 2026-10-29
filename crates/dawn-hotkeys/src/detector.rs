//! The double-press state machine. Pure and platform independent: it is fed key events with
//! timestamps and says when the shortcut fired.

use crate::shortcut::Shortcut;

/// How quickly the second press must follow the first.
pub const DEFAULT_WINDOW_MS: u64 = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    /// Windows virtual-key code.
    pub vk: u32,
    pub down: bool,
    /// Milliseconds, any monotonic clock (the keyboard hook's own timestamp).
    pub time_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Nothing,
    /// The first press of a double shortcut was seen (a good moment to note the clipboard state).
    FirstPress,
    Fired,
}

#[derive(Debug, Default, Clone, Copy)]
struct Held {
    ctrl_l: bool,
    ctrl_r: bool,
    alt_l: bool,
    alt_r: bool,
    shift_l: bool,
    shift_r: bool,
    win_l: bool,
    win_r: bool,
}

impl Held {
    /// Record a modifier key going down or up. Returns false if `vk` is not a modifier.
    fn update(&mut self, vk: u32, down: bool) -> bool {
        // The generic codes (0x10 Shift, 0x11 Ctrl, 0x12 Alt) are tracked as the left key.
        match vk {
            0x11 | 0xA2 => self.ctrl_l = down,
            0xA3 => self.ctrl_r = down,
            0x12 | 0xA4 => self.alt_l = down,
            0xA5 => self.alt_r = down,
            0x10 | 0xA0 => self.shift_l = down,
            0xA1 => self.shift_r = down,
            0x5B => self.win_l = down,
            0x5C => self.win_r = down,
            _ => return false,
        }
        true
    }

    /// Exactly the modifiers the shortcut asks for are held.
    fn matches(&self, s: &Shortcut) -> bool {
        (self.ctrl_l || self.ctrl_r) == s.ctrl
            && (self.alt_l || self.alt_r) == s.alt
            && (self.shift_l || self.shift_r) == s.shift
            && (self.win_l || self.win_r) == s.win
    }
}

#[derive(Debug, Clone)]
pub struct Detector {
    shortcut: Shortcut,
    window_ms: u64,
    held: Held,
    /// The trigger key is down (so auto-repeat does not count as another press).
    key_down: bool,
    first_press_at: Option<u64>,
}

impl Detector {
    pub fn new(shortcut: Shortcut, window_ms: u64) -> Self {
        Self { shortcut, window_ms, held: Held::default(), key_down: false, first_press_at: None }
    }

    pub fn shortcut(&self) -> &Shortcut {
        &self.shortcut
    }

    /// Feed one key event. A double shortcut fires on the second key-down inside the window,
    /// with the modifiers held (they may be released and pressed again in between). Any other
    /// key in between, a late second press or the wrong modifiers start over. After firing the
    /// next press counts as a new first press, so three quick presses fire once.
    pub fn on_event(&mut self, ev: KeyEvent) -> Outcome {
        if self.held.update(ev.vk, ev.down) {
            return Outcome::Nothing; // a modifier changed; it never ends a sequence
        }
        if ev.vk != self.shortcut.key.0 {
            if ev.down {
                self.first_press_at = None; // a different key in between
            }
            return Outcome::Nothing;
        }
        if !ev.down {
            self.key_down = false;
            return Outcome::Nothing;
        }
        if self.key_down {
            return Outcome::Nothing; // auto-repeat while the key is held
        }
        self.key_down = true;

        if !self.held.matches(&self.shortcut) {
            self.first_press_at = None;
            return Outcome::Nothing;
        }
        if !self.shortcut.double {
            return Outcome::Fired;
        }
        match self.first_press_at {
            Some(first) if ev.time_ms.wrapping_sub(first) <= self.window_ms => {
                self.first_press_at = None;
                Outcome::Fired
            }
            _ => {
                self.first_press_at = Some(ev.time_ms);
                Outcome::FirstPress
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTRL: u32 = 0xA2;
    const SHIFT: u32 = 0xA0;
    const C: u32 = 0x43;
    const V: u32 = 0x56;

    fn ev(vk: u32, down: bool, time_ms: u64) -> KeyEvent {
        KeyEvent { vk, down, time_ms }
    }

    fn double() -> Detector {
        Detector::new(Shortcut::ctrl_c_c(), DEFAULT_WINDOW_MS)
    }

    /// Feed `(key, down, time)` triples and collect the non-trivial outcomes.
    fn run(d: &mut Detector, events: &[(u32, bool, u64)]) -> Vec<Outcome> {
        events.iter().map(|&(vk, down, t)| d.on_event(ev(vk, down, t))).filter(|o| *o != Outcome::Nothing).collect()
    }

    #[test]
    fn ctrl_held_and_c_pressed_twice_fires() {
        let mut d = double();
        let out = run(&mut d, &[(CTRL, true, 0), (C, true, 10), (C, false, 60), (C, true, 200), (C, false, 250)]);
        assert_eq!(out, vec![Outcome::FirstPress, Outcome::Fired]);
    }

    #[test]
    fn ctrl_released_and_pressed_again_still_fires() {
        let mut d = double();
        let out = run(&mut d, &[(CTRL, true, 0), (C, true, 10), (C, false, 40), (CTRL, false, 60), (CTRL, true, 120), (C, true, 160)]);
        assert_eq!(out, vec![Outcome::FirstPress, Outcome::Fired]);
    }

    #[test]
    fn a_second_press_that_is_too_slow_starts_over() {
        let mut d = double();
        let out = run(&mut d, &[(CTRL, true, 0), (C, true, 10), (C, false, 50), (C, true, 511), (C, false, 560)]);
        assert_eq!(out, vec![Outcome::FirstPress, Outcome::FirstPress], "401 ms is outside the window");
        // ... and the late press is a new first press: a quick press after it fires.
        assert_eq!(d.on_event(ev(C, true, 700)), Outcome::Fired);
    }

    #[test]
    fn the_window_edge_is_inclusive() {
        let mut d = double();
        let out = run(&mut d, &[(CTRL, true, 0), (C, true, 100), (C, false, 150), (C, true, 500)]);
        assert_eq!(out, vec![Outcome::FirstPress, Outcome::Fired], "exactly 400 ms still counts");
    }

    #[test]
    fn a_different_key_in_between_cancels() {
        let mut d = double();
        let out = run(&mut d, &[(CTRL, true, 0), (C, true, 10), (C, false, 40), (V, true, 80), (V, false, 100), (C, true, 150)]);
        assert_eq!(out, vec![Outcome::FirstPress, Outcome::FirstPress]);
    }

    #[test]
    fn key_up_of_another_key_does_not_cancel() {
        let mut d = double();
        // The user may still be releasing an earlier key.
        let out = run(&mut d, &[(CTRL, true, 0), (C, true, 10), (C, false, 40), (V, false, 60), (C, true, 100)]);
        assert_eq!(out, vec![Outcome::FirstPress, Outcome::Fired]);
    }

    #[test]
    fn three_quick_presses_fire_once() {
        let mut d = double();
        let out = run(&mut d, &[(CTRL, true, 0), (C, true, 10), (C, false, 30), (C, true, 100), (C, false, 130), (C, true, 200), (C, false, 230)]);
        assert_eq!(out, vec![Outcome::FirstPress, Outcome::Fired, Outcome::FirstPress]);
        // A fourth press right after completes a second pair.
        assert_eq!(d.on_event(ev(C, true, 300)), Outcome::Fired);
    }

    #[test]
    fn auto_repeat_while_holding_c_does_not_count() {
        let mut d = double();
        let out = run(&mut d, &[(CTRL, true, 0), (C, true, 10), (C, true, 60), (C, true, 90), (C, true, 120), (C, false, 150)]);
        assert_eq!(out, vec![Outcome::FirstPress], "held key repeats are one press");
    }

    #[test]
    fn needs_exactly_the_right_modifiers() {
        let mut d = double();
        // No Ctrl at all.
        assert!(run(&mut d, &[(C, true, 0), (C, false, 20), (C, true, 100)]).is_empty());
        // Ctrl+Shift+C is a different shortcut.
        let mut d = double();
        assert!(run(&mut d, &[(CTRL, true, 0), (SHIFT, true, 5), (C, true, 10), (C, false, 30), (C, true, 100)]).is_empty());
    }

    #[test]
    fn a_single_press_shortcut_fires_on_every_press() {
        let mut d = Detector::new("Ctrl+F8".parse().unwrap(), DEFAULT_WINDOW_MS);
        let out = run(&mut d, &[(CTRL, true, 0), (0x77, true, 10), (0x77, false, 20), (0x77, true, 900), (0x77, false, 920)]);
        assert_eq!(out, vec![Outcome::Fired, Outcome::Fired]);
    }

    #[test]
    fn the_window_is_configurable_and_handles_clock_wrap() {
        let mut d = Detector::new(Shortcut::ctrl_c_c(), 150);
        let out = run(&mut d, &[(CTRL, true, 0), (C, true, 10), (C, false, 40), (C, true, 200)]);
        assert_eq!(out, vec![Outcome::FirstPress, Outcome::FirstPress], "190 ms is too slow for a 150 ms window");

        // The hook's millisecond clock wraps around after about 49 days.
        let mut d = double();
        let out = run(&mut d, &[(CTRL, true, u64::MAX - 20), (C, true, u64::MAX - 10), (C, false, u64::MAX - 5), (C, true, 90)]);
        assert_eq!(out, vec![Outcome::FirstPress, Outcome::Fired]);
    }
}
