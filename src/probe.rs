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

/// Panel-vs-framebuffer downscale ratio — physical panel px ÷ the framebuffer
/// px the OS renders into, for the main display.
///
/// `< 1.0` means the OS compositor DOWNSCALES the app's framebuffer to a
/// smaller physical panel (a macOS "scaled" / "More Space" mode) — the
/// row-seam condition an integer-cell terminal can't fix in its own
/// framebuffer. `1.0` means the framebuffer IS the panel grid (Default /
/// integer-Retina mode → no seam). `None` off-macOS or when the display
/// geometry can't be read; consumers treat `None` as 1.0 (no adjustment),
/// so a failed probe is always a safe no-op.
///
/// macOS-only today (CoreGraphics display modes); Wayland/X11/Windows land as
/// future `cfg` arms behind the same signature.
#[must_use]
pub fn display_scaling_ratio() -> Option<f32> {
    #[cfg(target_os = "macos")]
    {
        mac::display_scaling_ratio()
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

/// Whether the process runs inside a Kubernetes pod. `Some(true)` when the
/// in-cluster service-account is injected (`KUBERNETES_SERVICE_HOST` env or the
/// projected SA-token file), `Some(false)` when neither is present. Never
/// `None` — absence of the signals IS the answer. Cheap (env + one `stat`), no
/// network.
#[must_use]
pub fn in_cluster() -> Option<bool> {
    let env = std::env::var_os("KUBERNETES_SERVICE_HOST").is_some();
    let token =
        std::path::Path::new("/var/run/secrets/kubernetes.io/serviceaccount/token").exists();
    Some(env || token)
}

/// The cgroup memory limit in bytes — the real ceiling the workload runs under
/// (which a container often sees instead of host RAM). cgroup v2
/// `/sys/fs/cgroup/memory.max`, then v1
/// `/sys/fs/cgroup/memory/memory.limit_in_bytes`. `None` off-Linux, when
/// unreadable, or when unlimited (`"max"` / a sentinel ≥ host scale). Linux
/// only; the read is the documented filesystem reach.
#[must_use]
pub fn cgroup_memory_limit_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        // cgroup v2: a single value or the literal "max" (= unlimited).
        if let Ok(s) = std::fs::read_to_string("/sys/fs/cgroup/memory.max") {
            let t = s.trim();
            if t == "max" {
                return None;
            }
            if let Ok(v) = t.parse::<u64>() {
                return (v > 0).then_some(v);
            }
        }
        // cgroup v1: a huge sentinel (PAGE_COUNTER_MAX rounded) means unlimited.
        if let Ok(s) = std::fs::read_to_string("/sys/fs/cgroup/memory/memory.limit_in_bytes") {
            if let Ok(v) = s.trim().parse::<u64>() {
                // ~8 EiB sentinel ⇒ unlimited; treat as no limit.
                return (v > 0 && v < (1u64 << 62)).then_some(v);
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
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

    /// macOS tags the display's true panel mode with this IOKit flag; its
    /// pixel height is the physical panel resolution regardless of a
    /// supersampled scaled mode being active. (kDisplayModeNativeFlag.)
    pub(super) const NATIVE_MODE_FLAG: u32 = 0x0200_0000;

    /// Pure panel-height selection over the mode table — the testable heart of
    /// `display_scaling_ratio`. `modes` is `(pixel_height, is_native_flagged)`
    /// for every enumerated display mode; `fb_h` is the current framebuffer's
    /// pixel height. Returns the physical panel's pixel height, or `None` when
    /// the table carries no usable height.
    ///
    /// Selection order (empirically grounded on the built-in Liquid Retina XDR,
    /// live mode dump 2026-07-11 — see `examples/display_modes_dump.rs`):
    /// 1. If any mode carries the native flag, the panel is the **max native**
    ///    pixel height (the XDR reports its 3456×2234 panel this way; a
    ///    4112×2658 supersampled "More Space" framebuffer is a *non*-native
    ///    scaled mode, so it is correctly excluded).
    /// 2. Otherwise fall back to the **max pixel height across all modes** — on
    ///    a scaled display the panel mode is still the tallest present, and this
    ///    keeps the probe answering on a hypothetical display whose native flag
    ///    is absent rather than silently returning 1.0.
    ///
    /// The caller turns `panel_h / fb_h` into the downscale ratio.
    #[must_use]
    pub(super) fn select_panel_height(fb_h: usize, modes: &[(usize, bool)]) -> Option<usize> {
        let max_native = modes
            .iter()
            .filter(|(_, native)| *native)
            .map(|(h, _)| *h)
            .max()
            .filter(|h| *h > 0);
        if let Some(h) = max_native {
            return Some(h);
        }
        // No native-flagged mode: the panel is the tallest mode we can see.
        // Bound below by `fb_h` so a table that only lists scaled-down modes
        // never invents a downscale (panel < framebuffer would be an upscale,
        // which the ratio math treats as no-op anyway).
        modes
            .iter()
            .map(|(h, _)| *h)
            .chain(std::iter::once(fb_h))
            .max()
            .filter(|h| *h > 0)
    }

    /// Turn a selected panel height + framebuffer height into a plausible
    /// downscale ratio, or `None` if implausible. Extracted so the numeric
    /// tail is unit-testable independent of CoreGraphics.
    #[must_use]
    pub(super) fn ratio_from_heights(panel_h: usize, fb_h: usize) -> Option<f32> {
        if panel_h == 0 || fb_h == 0 {
            return None;
        }
        #[allow(clippy::cast_precision_loss)]
        let ratio = panel_h as f32 / fb_h as f32;
        // Trust only a plausible ratio. The consumer clamps to [0.25, 1.0] and
        // only snaps when < 1.0 (a downscale), so an integer/Default mode
        // (ratio 1.0) and an upscale (> 1.0) both no-op safely.
        (ratio.is_finite() && (0.3..=3.0).contains(&ratio)).then_some(ratio)
    }

    pub fn display_scaling_ratio() -> Option<f32> {
        use objc2_core_foundation::{CFArrayGetCount, CFArrayGetValueAtIndex};
        use objc2_core_graphics::{
            CGDisplayCopyAllDisplayModes, CGDisplayCopyDisplayMode, CGDisplayMode,
            CGDisplayModeGetIOFlags, CGDisplayModeGetPixelHeight, CGMainDisplayID,
        };

        let display = CGMainDisplayID();

        // The framebuffer the OS renders into = the CURRENT mode's pixel height.
        let current = CGDisplayCopyDisplayMode(display)?;
        let fb_h = CGDisplayModeGetPixelHeight(Some(&current));
        if fb_h == 0 {
            return None;
        }

        // Collect (pixel_height, is_native) for every mode, then select the
        // panel height purely (testable). SAFETY: `None` options requests the
        // default mode list; the returned CFArray owns its CGDisplayMode
        // values, valid while `modes` lives.
        let modes = unsafe { CGDisplayCopyAllDisplayModes(display, None) }?;
        let count = CFArrayGetCount(&modes);
        let mut table: Vec<(usize, bool)> = Vec::with_capacity(count.max(0) as usize);
        for i in 0..count {
            // SAFETY: 0 <= i < count; the value is a CGDisplayModeRef
            // borrowed from the retained array for the loop body only.
            let ptr = unsafe { CFArrayGetValueAtIndex(&modes, i) }.cast::<CGDisplayMode>();
            if ptr.is_null() {
                continue;
            }
            let mode = unsafe { &*ptr };
            let h = CGDisplayModeGetPixelHeight(Some(mode));
            let native = CGDisplayModeGetIOFlags(Some(mode)) & NATIVE_MODE_FLAG != 0;
            table.push((h, native));
        }

        let panel_h = select_panel_height(fb_h, &table)?;
        ratio_from_heights(panel_h, fb_h)
    }
}

#[cfg(all(test, target_os = "macos"))]
mod display_probe_tests {
    use super::mac::{ratio_from_heights, select_panel_height};

    /// The downscale ratio is either `None` (headless / off-macOS) or a
    /// plausible panel/framebuffer ratio — never a garbage value that could
    /// mis-snap a consumer's cell grid.
    #[test]
    fn display_scaling_ratio_is_none_or_plausible() {
        if let Some(r) = super::display_scaling_ratio() {
            assert!((0.3..=3.0).contains(&r), "implausible ratio {r}");
            eprintln!("display_scaling_ratio() = {r}");
        } else {
            eprintln!("display_scaling_ratio() = None (headless / no display)");
        }
    }

    /// The exact mode table dumped live from the built-in Liquid Retina XDR on
    /// the "More Space" scaling (2026-07-11; `examples/display_modes_dump.rs`):
    /// a 4112×2658 supersampled framebuffer (current, NON-native) downscaled to
    /// the physical 3456×2234 panel (the native-flagged mode). Only the panel
    /// mode carries the native flag; every scaled mode does not. The probe must
    /// therefore pick 2234 (panel), NOT 2658 (framebuffer), yielding 0.8404.
    #[test]
    fn xdr_more_space_selects_physical_panel_not_framebuffer() {
        // (pixel_height, is_native) — abridged to the distinct heights present
        // in the live dump; every non-native scaled height plus the 6 native
        // 2234 rows. fb_h is the current framebuffer's pixel height (2658).
        let modes: Vec<(usize, bool)> = vec![
            (1200, false),
            (1460, false),
            (1510, false),
            (1600, false),
            (1640, false),
            (1696, false),
            (1870, false),
            (1934, false),
            (2160, false),
            (2234, true), // the native-flagged physical panel
        ];
        let fb_h = 2658;
        let panel_h = select_panel_height(fb_h, &modes).expect("panel height");
        assert_eq!(panel_h, 2234, "must pick the native-flagged physical panel");
        let ratio = ratio_from_heights(panel_h, fb_h).expect("ratio");
        assert!(
            (ratio - 0.840_481_6).abs() < 1.0e-4,
            "XDR More-Space ratio must be ~0.8405, got {ratio}"
        );
    }

    /// A conventional Default / integer-Retina 2× display: the framebuffer IS
    /// the panel grid, so the current mode carries the native flag and its
    /// pixel height equals the framebuffer. Ratio must be exactly 1.0 — the
    /// integer-scale path stays untouched (no seam snap engages).
    #[test]
    fn default_retina_2x_yields_unity_ratio() {
        // A typical MBP built-in at Default scaling: native mode == framebuffer.
        let modes: Vec<(usize, bool)> = vec![
            (900, false),  // scaled-down 1x-ish choices
            (1050, false),
            (1200, false),
            (1600, true), // native panel == current framebuffer
        ];
        let fb_h = 1600;
        let panel_h = select_panel_height(fb_h, &modes).expect("panel height");
        assert_eq!(panel_h, 1600);
        let ratio = ratio_from_heights(panel_h, fb_h).expect("ratio");
        assert!(
            (ratio - 1.0).abs() < 1.0e-6,
            "default 2x must yield unity ratio, got {ratio}"
        );
    }

    /// Fallback: a display whose native flag is absent from the table still
    /// answers, picking the tallest mode as the panel rather than returning
    /// 1.0 by giving up. Here the panel (2234) is present but unflagged.
    #[test]
    fn fallback_picks_tallest_when_no_native_flag() {
        let modes: Vec<(usize, bool)> = vec![
            (1600, false),
            (2160, false),
            (2234, false), // panel, but the native flag is (hypothetically) missing
        ];
        let fb_h = 2658;
        let panel_h = select_panel_height(fb_h, &modes).expect("panel height");
        assert_eq!(panel_h, 2658.max(2234)); // bounded below by fb_h → 2658
        // With fb bounding, panel==fb → unity (no invented downscale). Honest:
        // without the native flag we cannot distinguish a downscale, so we
        // decline to snap rather than guess a seam correction.
        let ratio = ratio_from_heights(panel_h, fb_h).expect("ratio");
        assert!((ratio - 1.0).abs() < 1.0e-6);
    }

    /// An upscaled (rare) or degenerate table never produces a snap-triggering
    /// ratio below 1.0 from garbage; implausible ratios return `None`.
    #[test]
    fn implausible_ratio_is_rejected() {
        assert!(ratio_from_heights(0, 2658).is_none());
        assert!(ratio_from_heights(2234, 0).is_none());
        // A wildly-out-of-range ratio (10000 panel / 100 fb) is rejected.
        assert!(ratio_from_heights(10_000, 100).is_none());
    }
}
