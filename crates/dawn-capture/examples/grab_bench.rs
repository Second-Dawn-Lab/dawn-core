//! How much a grab costs: the whole monitor against a caption-sized region.
//!
//! ```text
//! cargo run --release -p dawn-capture --example grab_bench
//! ```

use std::time::Instant;

use dawn_capture::{Grabber, Region};

fn main() {
    let monitors = xcap::Monitor::all().expect("monitors");
    let m = monitors.iter().find(|m| m.is_primary().unwrap_or(false)).unwrap_or(&monitors[0]);
    let (mx, my, mw, mh) = (m.x().unwrap(), m.y().unwrap(), m.width().unwrap(), m.height().unwrap());
    println!("monitor {mw}x{mh} at ({mx}, {my})");

    // `grab_bench full 200` or `grab_bench region 600`: only that mode, so a process timer shows its CPU time.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [mode, count] = args.as_slice() {
        let count: usize = count.parse().unwrap_or(100);
        let region = Region { x: mx + 200, y: my + mh as i32 - 300, w: 1000, h: 150 };
        let grabber = Grabber::new(region).unwrap();
        let started = Instant::now();
        for _ in 0..count {
            if mode == "full" {
                let _ = m.capture_image().unwrap();
            } else {
                let _ = grabber.grab().unwrap();
            }
        }
        println!("{mode}: {count} grabs in {:.0} ms", started.elapsed().as_secs_f64() * 1000.0);
        return;
    }
    let n = 60;
    let started = Instant::now();
    for _ in 0..n {
        let _ = m.capture_image().unwrap();
    }
    println!("whole monitor : {:.1} ms per grab", started.elapsed().as_secs_f64() * 1000.0 / n as f64);

    for (w, h) in [(1000u32, 150u32), (1600, 220), (600, 100)] {
        let region = Region { x: mx + (mw as i32 - w as i32) / 2, y: my + mh as i32 - h as i32 - 120, w, h };
        let grabber = Grabber::new(region).unwrap();
        let started = Instant::now();
        for _ in 0..n * 4 {
            let _ = grabber.grab().unwrap();
        }
        println!("region {w}x{h:<4}: {:.2} ms per grab", started.elapsed().as_secs_f64() * 1000.0 / (n * 4) as f64);
    }
}
