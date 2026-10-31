//! Where the caption bar goes: just below the watched region, on screen, on whichever monitor fits. All in physical pixels
//! of the virtual desktop (monitors can sit left of or above the primary one, so coordinates may be negative).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> i32 {
        self.x + self.w as i32
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h as i32
    }

    fn center(&self) -> (i32, i32) {
        (self.x + self.w as i32 / 2, self.y + self.h as i32 / 2)
    }

    fn contains(&self, p: (i32, i32)) -> bool {
        p.0 >= self.x && p.0 < self.right() && p.1 >= self.y && p.1 < self.bottom()
    }

    fn overlap(&self, other: &Rect) -> i64 {
        let w = (self.right().min(other.right()) - self.x.max(other.x)).max(0) as i64;
        let h = (self.bottom().min(other.bottom()) - self.y.max(other.y)).max(0) as i64;
        w * h
    }
}

/// The monitor a rectangle is on: the one under its center, else the one it overlaps most, else the first.
fn monitor_of<'a>(rect: &Rect, monitors: &'a [Rect]) -> Option<&'a Rect> {
    monitors.iter().find(|m| m.contains(rect.center())).or_else(|| monitors.iter().max_by_key(|m| m.overlap(rect)).filter(|m| m.overlap(rect) > 0)).or_else(|| monitors.first())
}

/// The bar's rectangle: centered under the region (above it when there is no room below, inside its bottom edge when there is
/// room on neither side), always inside the monitor of the region. A bar larger than the monitor is made to fit.
pub fn place_bar(region: Rect, bar_w: u32, bar_h: u32, monitors: &[Rect], gap: i32) -> Rect {
    let Some(m) = monitor_of(&region, monitors) else { return Rect::new(region.x, region.bottom() + gap, bar_w, bar_h) };
    let w = bar_w.min(m.w);
    let h = bar_h.min(m.h);
    let x = (region.x + (region.w as i32 - w as i32) / 2).clamp(m.x, m.right() - w as i32);
    let below = region.bottom() + gap;
    let above = region.y - gap - h as i32;
    let y = if below + h as i32 <= m.bottom() {
        below
    } else if above >= m.y {
        above
    } else {
        (region.bottom() - h as i32).clamp(m.y, m.bottom() - h as i32)
    };
    Rect::new(x, y, w, h)
}

/// Where a small list window goes: under the control it belongs to, as wide as the control at least, flipped above it when
/// there is no room below, and always inside the monitor of the control (a list taller than the room is shortened to fit).
pub fn place_popup(anchor: Rect, width: u32, height: u32, monitors: &[Rect], gap: i32) -> Rect {
    let Some(m) = monitor_of(&anchor, monitors) else { return Rect::new(anchor.x, anchor.bottom() + gap, width, height) };
    let w = width.max(anchor.w).min(m.w);
    let x = anchor.x.clamp(m.x, (m.right() - w as i32).max(m.x));
    let below_room = (m.bottom() - (anchor.bottom() + gap)).max(0) as u32;
    let above_room = (anchor.y - gap - m.y).max(0) as u32;
    // Below when it fits, else above when it fits, else on the side with more room, shortened.
    if height <= below_room {
        Rect::new(x, anchor.bottom() + gap, w, height)
    } else if height <= above_room {
        Rect::new(x, anchor.y - gap - height as i32, w, height)
    } else if below_room >= above_room {
        Rect::new(x, anchor.bottom() + gap, w, below_room.max(60).min(height))
    } else {
        let h = above_room.max(60).min(height);
        Rect::new(x, anchor.y - gap - h as i32, w, h)
    }
}

/// A size in logical pixels as physical pixels on a monitor with this scale (1.0 = 100 %).
pub fn physical(logical: u32, scale: f64) -> u32 {
    (logical as f64 * scale).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRIMARY: Rect = Rect { x: 0, y: 0, w: 1920, h: 1080 };
    const RIGHT: Rect = Rect { x: 1920, y: 0, w: 2560, h: 1440 };
    const LEFT: Rect = Rect { x: -1280, y: 100, w: 1280, h: 1024 };

    #[test]
    fn the_bar_sits_centered_just_below_the_region() {
        let bar = place_bar(Rect::new(400, 700, 800, 100), 600, 120, &[PRIMARY], 8);
        assert_eq!(bar, Rect::new(500, 808, 600, 120));
    }

    #[test]
    fn near_the_bottom_it_goes_above_the_region() {
        let bar = place_bar(Rect::new(400, 960, 800, 100), 600, 120, &[PRIMARY], 8);
        assert_eq!(bar.y, 960 - 8 - 120);
    }

    #[test]
    fn a_region_filling_the_screen_gets_the_bar_inside_its_bottom_edge() {
        let bar = place_bar(Rect::new(0, 0, 1920, 1080), 600, 120, &[PRIMARY], 8);
        assert_eq!((bar.y, bar.y + bar.h as i32), (960, 1080));
    }

    #[test]
    fn the_bar_is_kept_inside_the_monitor_horizontally() {
        let bar = place_bar(Rect::new(1800, 100, 100, 50), 600, 120, &[PRIMARY], 8);
        assert_eq!(bar.x + bar.w as i32, 1920);
        let bar = place_bar(Rect::new(0, 100, 50, 50), 600, 120, &[PRIMARY], 8);
        assert_eq!(bar.x, 0);
    }

    #[test]
    fn it_follows_the_region_to_another_monitor_even_with_negative_coordinates() {
        let monitors = [PRIMARY, RIGHT, LEFT];
        let on_right = place_bar(Rect::new(2500, 1300, 800, 100), 600, 120, &monitors, 8);
        assert!(on_right.x >= 1920 && on_right.right() <= 4480 && on_right.y < 1300, "{on_right:?}");
        let on_left = place_bar(Rect::new(-1000, 900, 600, 100), 500, 100, &monitors, 8);
        assert!(on_left.x >= -1280 && on_left.right() <= 0 && on_left.bottom() <= 1124, "{on_left:?}");
    }

    #[test]
    fn a_region_spanning_two_monitors_uses_the_one_under_its_center_and_a_bar_wider_than_it_shrinks() {
        let monitors = [PRIMARY, RIGHT];
        let bar = place_bar(Rect::new(1700, 200, 600, 100), 3000, 100, &monitors, 8);
        assert!(bar.w <= 2560 && bar.x >= 1920, "{bar:?}");
    }

    #[test]
    fn mixed_dpi_sizes_are_converted_per_monitor() {
        assert_eq!(physical(600, 1.0), 600);
        assert_eq!(physical(600, 1.5), 900);
        assert_eq!(physical(120, 1.25), 150);
        // 600 logical px at 150 % is 900 physical: placed under a region on that monitor.
        let hidpi = Rect::new(0, 0, 3840, 2160);
        let bar = place_bar(Rect::new(1000, 1500, 1500, 200), physical(600, 1.5), physical(120, 1.5), &[hidpi], 12);
        assert_eq!((bar.w, bar.h), (900, 180));
        assert_eq!(bar.y, 1712);
    }

    #[test]
    fn no_monitors_still_gives_a_rectangle() {
        let bar = place_bar(Rect::new(10, 10, 100, 50), 300, 80, &[], 8);
        assert_eq!((bar.x, bar.y), (10, 68));
    }

    #[test]
    fn a_list_goes_under_its_control_or_above_it_and_stays_on_the_monitor() {
        let below = place_popup(Rect::new(100, 200, 160, 30), 200, 300, &[PRIMARY], 4);
        assert_eq!(below, Rect::new(100, 234, 200, 300));
        // Near the bottom: above.
        let above = place_popup(Rect::new(100, 1000, 160, 30), 200, 300, &[PRIMARY], 4);
        assert_eq!((above.y, above.bottom()), (1000 - 4 - 300, 1000 - 4));
        // At the right edge: pulled left to fit.
        let edge = place_popup(Rect::new(1850, 200, 60, 30), 200, 300, &[PRIMARY], 4);
        assert_eq!(edge.right(), 1920);
        // As wide as the control at least.
        assert_eq!(place_popup(Rect::new(10, 10, 400, 30), 200, 100, &[PRIMARY], 4).w, 400);
    }

    #[test]
    fn a_list_taller_than_the_room_is_shortened_on_the_roomier_side_and_follows_other_monitors() {
        // The bar sits at the bottom of a short monitor: more room above than below.
        let short = Rect::new(0, 0, 1920, 400);
        let tall = place_popup(Rect::new(100, 300, 100, 30), 200, 500, &[short], 4);
        assert!(tall.h < 500 && tall.y >= 0 && tall.bottom() <= 400, "{tall:?}");
        let on_right = place_popup(Rect::new(2500, 600, 120, 30), 200, 300, &[PRIMARY, RIGHT], 4);
        assert!(on_right.x >= 1920 && on_right.right() <= 4480, "{on_right:?}");
        let on_left = place_popup(Rect::new(-500, 300, 120, 30), 200, 300, &[PRIMARY, LEFT], 4);
        assert!(on_left.x >= -1280 && on_left.right() <= 0, "{on_left:?}");
    }
}
