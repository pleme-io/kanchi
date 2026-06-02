//! Platform environment probes — cfg-gated FFI, extracted once for the
//! whole fleet so no consumer re-vendors the NSScreen / sysctl / `/proc`
//! dance. Every probe returns `None` off-platform / when unanswerable, so
//! the `defaxes!`-generated resolver lands cleanly on the fallback.

/// Display-fit window dims: `frac` of the focused display's visible area,
/// clamped to `[min, max]`. macOS only (NSScreen, main thread); `None`
/// elsewhere or off the main thread.
#[must_use]
pub fn screen_frac(frac: f64, min: (u32, u32), max: (u32, u32)) -> Option<(u32, u32)> {
    #[cfg(target_os = "macos")]
    {
        mac::screen_frac(frac, min, max)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (frac, min, max);
        None
    }
}

/// DPR-aware font size: HiDPI (≥1.5× backing scale, the OS upscales) →
/// `hidpi_pt`; low-DPI → `lodpi_pt`. macOS only.
#[must_use]
pub fn dpr_font_size(hidpi_pt: f32, lodpi_pt: f32) -> Option<f32> {
    #[cfg(target_os = "macos")]
    {
        mac::dpr_font_size(hidpi_pt, lodpi_pt)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (hidpi_pt, lodpi_pt);
        None
    }
}

/// Whether the host is in dark appearance. macOS reads
/// `AppleInterfaceStyle` (present ⇒ dark, absent ⇒ light). `None` on other
/// platforms until a portable (xdg-portal / OSC 11) query lands.
#[must_use]
pub fn appearance_dark() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        mac::appearance_dark()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Total physical RAM in GiB. macOS `sysctl hw.memsize`; Linux
/// `/proc/meminfo`; `None` elsewhere.
#[must_use]
pub fn total_ram_gib() -> Option<u64> {
    total_ram_bytes().map(|b| b / (1024 * 1024 * 1024))
}

#[cfg(target_os = "macos")]
fn total_ram_bytes() -> Option<u64> {
    let mut size: u64 = 0;
    let mut len = core::mem::size_of::<u64>();
    let name = c"hw.memsize";
    // SAFETY: valid NUL-terminated key; size/len sized for a u64 out-param.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            core::ptr::addr_of_mut!(size).cast(),
            &mut len,
            core::ptr::null_mut(),
            0,
        )
    };
    (rc == 0 && size > 0).then_some(size)
}

#[cfg(target_os = "linux")]
fn total_ram_bytes() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb: u64 = meminfo
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))?
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()?;
    Some(kb * 1024)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn total_ram_bytes() -> Option<u64> {
    None
}

#[cfg(target_os = "macos")]
mod mac {
    use crate::clamp;
    use objc2_app_kit::NSScreen;
    use objc2_foundation::{MainThreadMarker, NSString, NSUserDefaults};

    pub fn screen_frac(frac: f64, min: (u32, u32), max: (u32, u32)) -> Option<(u32, u32)> {
        let mtm = MainThreadMarker::new()?;
        let screen = NSScreen::mainScreen(mtm)?;
        let frame = screen.visibleFrame();
        let (w, h) = (frame.size.width.max(0.0), frame.size.height.max(0.0));
        if w < 200.0 || h < 200.0 {
            return None;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (pw, ph) = ((w * frac).round() as u32, (h * frac).round() as u32);
        Some((clamp(pw, min.0, max.0), clamp(ph, min.1, max.1)))
    }

    pub fn dpr_font_size(hidpi_pt: f32, lodpi_pt: f32) -> Option<f32> {
        let mtm = MainThreadMarker::new()?;
        let screen = NSScreen::mainScreen(mtm)?;
        let scale = screen.backingScaleFactor();
        if scale <= 0.0 {
            return None;
        }
        Some(if scale >= 1.5 { hidpi_pt } else { lodpi_pt })
    }

    pub fn appearance_dark() -> Option<bool> {
        let defaults = NSUserDefaults::standardUserDefaults();
        let key = NSString::from_str("AppleInterfaceStyle");
        let style = defaults.stringForKey(&key);
        Some(style.is_some_and(|s| s.to_string().eq_ignore_ascii_case("dark")))
    }
}
