//! Screen capture and on-screen area selection.
//!
//! Freeze-frame design: when the shortcut fires, every monitor is captured first, then one
//! overlay per monitor shows its frozen screenshot, dimmed. The selection is cropped from the
//! stored screenshot, so the overlay never appears in the result and moving content (a video)
//! is frozen at the moment of the key press. Screenshots live in memory only.
//!
//! The coordinate mapping is in [`map`] (pure, unit-tested for mixed-DPI setups).
//! Exclusive-fullscreen games may not be capturable by the screen-capture APIs used here.

use anyhow::{anyhow, Result};

pub mod map;

pub use map::{Frame, PixelRect};

/// One monitor and its screenshot.
pub struct Screen {
    /// Position of the monitor on the virtual desktop, physical pixels.
    pub x: i32,
    pub y: i32,
    /// Physical size.
    pub width: u32,
    pub height: u32,
    /// DPI scale (1.0 = 100 %, 1.5 = 150 %).
    pub scale: f64,
    pub primary: bool,
    pub frame: Frame,
}

/// Capture every monitor, in the order the system reports them.
pub fn capture_all() -> Result<Vec<Screen>> {
    let monitors = xcap::Monitor::all().map_err(|e| anyhow!("could not list monitors: {e}"))?;
    let mut screens = Vec::new();
    for m in monitors {
        let image = m.capture_image().map_err(|e| anyhow!("could not capture a monitor: {e}"))?;
        screens.push(Screen {
            x: m.x().map_err(|e| anyhow!("{e}"))?,
            y: m.y().map_err(|e| anyhow!("{e}"))?,
            width: m.width().map_err(|e| anyhow!("{e}"))?,
            height: m.height().map_err(|e| anyhow!("{e}"))?,
            scale: m.scale_factor().map_err(|e| anyhow!("{e}"))? as f64,
            primary: m.is_primary().unwrap_or(false),
            frame: Frame { width: image.width(), height: image.height(), rgba: image.into_raw() },
        });
    }
    if screens.is_empty() {
        return Err(anyhow!("no monitor found"));
    }
    Ok(screens)
}
