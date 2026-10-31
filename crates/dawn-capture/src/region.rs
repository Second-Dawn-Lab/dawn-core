//! Grabbing a small region of the screen again and again (live captions).
//!
//! `xcap` on Windows copies just the requested rectangle from the screen (GDI `BitBlt` of that rectangle), so a grab costs
//! in proportion to the region, not to the monitor. See `docs/live-captions.md` for the measured costs against grabbing the
//! whole monitor. Windows that asked to be excluded from capture (`WDA_EXCLUDEFROMCAPTURE`) are not in the pixels.

use anyhow::{anyhow, Result};

use crate::map::Frame;

/// A rectangle of the virtual desktop in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// Grabs one region of one monitor.
pub struct Grabber {
    monitor: xcap::Monitor,
    /// The region relative to its monitor.
    rx: u32,
    ry: u32,
    w: u32,
    h: u32,
}

impl Grabber {
    /// The monitor under the region's center, and the region clipped to it.
    pub fn new(region: Region) -> Result<Self> {
        let monitors = xcap::Monitor::all().map_err(|e| anyhow!("could not list monitors: {e}"))?;
        let center = (region.x + region.w as i32 / 2, region.y + region.h as i32 / 2);
        for m in monitors {
            let (mx, my) = (m.x().map_err(|e| anyhow!("{e}"))?, m.y().map_err(|e| anyhow!("{e}"))?);
            let (mw, mh) = (m.width().map_err(|e| anyhow!("{e}"))?, m.height().map_err(|e| anyhow!("{e}"))?);
            if center.0 < mx || center.0 >= mx + mw as i32 || center.1 < my || center.1 >= my + mh as i32 {
                continue;
            }
            let (x0, y0) = ((region.x - mx).max(0) as u32, (region.y - my).max(0) as u32);
            let (x1, y1) = (((region.x + region.w as i32 - mx).max(0) as u32).min(mw), ((region.y + region.h as i32 - my).max(0) as u32).min(mh));
            if x1 <= x0 + 1 || y1 <= y0 + 1 {
                return Err(anyhow!("the region is outside the monitor"));
            }
            return Ok(Self { monitor: m, rx: x0, ry: y0, w: x1 - x0, h: y1 - y0 });
        }
        Err(anyhow!("the region is not on any monitor"))
    }

    pub fn size(&self) -> (u32, u32) {
        (self.w, self.h)
    }

    pub fn grab(&self) -> Result<Frame> {
        let image = self.monitor.capture_region(self.rx, self.ry, self.w, self.h).map_err(|e| anyhow!("could not grab the region: {e}"))?;
        Ok(Frame { width: image.width(), height: image.height(), rgba: image.into_raw() })
    }
}
