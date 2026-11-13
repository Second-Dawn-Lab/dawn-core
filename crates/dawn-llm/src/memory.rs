//! Memory checks before a model is loaded. A model that does not fit must give a clear message, not a crash.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryInfo {
    pub total: u64,
    pub available: u64,
}

const GIB: u64 = 1 << 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryVerdict {
    Ok,
    /// Loads, but the computer has less than 16 GB: show a warning.
    LowTotal,
    /// Clearly too little free memory: do not load.
    TooLittleFree { need: u64, free: u64 },
}

/// `need` is the pack's `ramBytes`. A model is refused when less than 85 % of what it needs is free (the weights are
/// memory-mapped, so a small shortfall only costs speed); below 16 GB of installed memory the user is warned.
pub fn check_memory(need: u64, info: MemoryInfo) -> MemoryVerdict {
    if info.available < need / 100 * 85 {
        return MemoryVerdict::TooLittleFree { need, free: info.available };
    }
    // Reported totals are a little under the nominal size (a "16 GB" laptop shows 15.8).
    if info.total < 16 * GIB - GIB / 2 {
        return MemoryVerdict::LowTotal;
    }
    MemoryVerdict::Ok
}

#[cfg(windows)]
pub fn system_memory() -> Option<MemoryInfo> {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    // SAFETY: `status` is a valid, correctly sized MEMORYSTATUSEX.
    unsafe { GlobalMemoryStatusEx(&mut status).ok()? };
    Some(MemoryInfo { total: status.ullTotalPhys, available: status.ullAvailPhys })
}

#[cfg(not(windows))]
pub fn system_memory() -> Option<MemoryInfo> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(total_gb: f64, free_gb: f64) -> MemoryInfo {
        MemoryInfo { total: (total_gb * GIB as f64) as u64, available: (free_gb * GIB as f64) as u64 }
    }

    #[test]
    fn plenty_of_memory_is_fine() {
        assert_eq!(check_memory(3 * GIB, info(16.0, 9.0)), MemoryVerdict::Ok);
        assert_eq!(check_memory(3 * GIB, info(15.8, 9.0)), MemoryVerdict::Ok, "a 16 GB laptop reports slightly less");
    }

    #[test]
    fn under_16_gb_warns_but_still_loads() {
        assert_eq!(check_memory(3 * GIB, info(8.0, 5.0)), MemoryVerdict::LowTotal);
    }

    #[test]
    fn clearly_too_little_free_memory_refuses() {
        match check_memory(3 * GIB, info(16.0, 1.5)) {
            MemoryVerdict::TooLittleFree { need, free } => {
                assert_eq!(need, 3 * GIB);
                assert!(free < need);
            }
            other => panic!("{other:?}"),
        }
        // A small shortfall is allowed (memory-mapped weights).
        assert_eq!(check_memory(3 * GIB, info(16.0, 2.7)), MemoryVerdict::Ok);
    }

    #[test]
    fn the_system_reports_some_memory_on_windows() {
        if cfg!(windows) {
            let m = system_memory().unwrap();
            assert!(m.total > 0 && m.available <= m.total);
        }
    }
}
