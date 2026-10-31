//! How often to look at the region.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Speed {
    Calm,
    #[default]
    Normal,
    Fast,
}

impl Speed {
    /// From the setting's text ("calm", "normal", "fast"); anything else is normal.
    pub fn parse(s: &str) -> Self {
        match s {
            "calm" => Self::Calm,
            "fast" => Self::Fast,
            _ => Self::Normal,
        }
    }

    fn base_ms(self) -> u64 {
        match self {
            Self::Calm => 800,
            Self::Normal => 500,
            Self::Fast => 250,
        }
    }
}

/// Never wait longer than this: a caption that appears must show up within a few seconds.
const MAX_MS: u64 = 3000;

/// The time to the next look. `idle_ms`: how long nothing changed. `on_battery`: running on battery power.
/// `last_work_ms`: how long the last read took; the loop never works more than about a third of the time, which keeps
/// the CPU use bounded whatever the speed setting is.
pub fn interval_ms(speed: Speed, idle_ms: u64, on_battery: bool, last_work_ms: u64) -> u64 {
    let mut ms = speed.base_ms();
    ms = match idle_ms {
        0..=4_999 => ms,
        5_000..=14_999 => ms * 3 / 2,
        15_000..=59_999 => ms * 2,
        _ => ms * 3,
    };
    if on_battery {
        ms = ms * 8 / 5;
    }
    ms.max(last_work_ms * 3).min(MAX_MS.max(last_work_ms * 3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_speed_setting_sets_the_base_rate() {
        assert_eq!(interval_ms(Speed::Calm, 0, false, 0), 800);
        assert_eq!(interval_ms(Speed::Normal, 0, false, 0), 500);
        assert_eq!(interval_ms(Speed::Fast, 0, false, 0), 250);
        assert_eq!(Speed::parse("fast"), Speed::Fast);
        assert_eq!(Speed::parse("nonsense"), Speed::Normal);
    }

    #[test]
    fn it_slows_down_while_nothing_changes_and_on_battery() {
        let at = |idle| interval_ms(Speed::Normal, idle, false, 0);
        assert!(at(0) < at(6_000) && at(6_000) < at(20_000) && at(20_000) < at(120_000));
        assert!(interval_ms(Speed::Normal, 0, true, 0) > at(0));
        assert!(interval_ms(Speed::Calm, 600_000, true, 0) <= 3000, "never too slow to notice a caption");
    }

    #[test]
    fn slow_work_stretches_the_interval_so_the_cpu_share_stays_bounded() {
        // A read that takes 400 ms (slow OCR) leaves at least 800 ms to rest: about a third of the time at work.
        assert!(interval_ms(Speed::Fast, 0, false, 400) >= 1200);
        assert!(interval_ms(Speed::Fast, 0, false, 20) == 250);
    }
}
