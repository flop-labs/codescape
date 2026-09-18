//! Resident memory of this process, for the benchmark's report.

/// Resident set size, in bytes.
#[derive(Clone, Copy, Debug)]
pub struct Resident {
    pub now: usize,
    pub peak: usize,
}

/// Reads the resident set, or `None` where this platform has no reader, so a
/// missing figure is reported as missing rather than as zero.
#[cfg(target_os = "linux")]
pub fn resident() -> Option<Resident> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb = |key: &str| -> Option<usize> {
        status
            .lines()
            .find(|l| l.starts_with(key))?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()
    };
    Some(Resident {
        now: kb("VmRSS:")? * 1024,
        peak: kb("VmHWM:")? * 1024,
    })
}

/// macOS has no `/proc`: `ps` reports the current set in KiB, and
/// `getrusage` the peak — in bytes here, where Linux uses KiB.
#[cfg(target_os = "macos")]
pub fn resident() -> Option<Resident> {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    let now_kb: usize = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    Some(Resident {
        now: now_kb * 1024,
        peak: peak_bytes()?,
    })
}

#[cfg(target_os = "macos")]
fn peak_bytes() -> Option<usize> {
    // `struct rusage` on 64-bit Darwin: two 16-byte timevals, then 14 longs,
    // of which ru_maxrss is the first.
    #[repr(C)]
    struct Rusage {
        times: [i64; 4],
        maxrss: i64,
        rest: [i64; 13],
    }
    extern "C" {
        fn getrusage(who: i32, usage: *mut Rusage) -> i32;
    }
    const RUSAGE_SELF: i32 = 0;
    let mut r = Rusage {
        times: [0; 4],
        maxrss: 0,
        rest: [0; 13],
    };
    // SAFETY: getrusage writes one `struct rusage`, which `Rusage` matches.
    let ok = unsafe { getrusage(RUSAGE_SELF, &mut r) } == 0;
    ok.then_some(r.maxrss as usize)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn resident() -> Option<Resident> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn resident_set_is_read_in_bytes() {
        // Touch 64 MiB so the set is well clear of noise.
        let block = vec![1u8; 64 << 20];
        let r = resident().expect("a resident-memory reader on this platform");
        std::hint::black_box(&block);
        assert!(r.now >= 64 << 20, "now {} bytes", r.now);
        // A unit slip (KiB read as bytes, or the reverse) is off by 1024x,
        // which these bounds catch in both directions.
        assert!(r.now < 1 << 40, "now {} bytes", r.now);
        assert!(r.peak >= r.now, "peak {} < now {}", r.peak, r.now);
        assert!(r.peak < 1 << 40, "peak {} bytes", r.peak);
    }
}
