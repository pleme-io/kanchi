# kanchi (感知 — "sensing / perception")

Typed **environment-discovery primitive** for the pleme-io fleet — the
shikumi `discovered()` tier made *declarative*.

Every GPU/terminal app faces the same domain: probe the host for the best-fit
value, fall back to a documented default when it can't, expose the result as a
discoverable tier. Hand-rolled, that's the same trio per axis, in every app:

```rust
pub const FALLBACK_X: T = ...;
pub fn detect_x() -> Option<T> { /* subtle, re-vendored platform FFI */ }
pub fn detect_x_or_fallback() -> T { detect_x().unwrap_or(FALLBACK_X) }
```

`kanchi` collapses the whole domain into a declaration — and the platform FFI
(NSScreen, `sysctl hw.memsize`, `/proc/meminfo`, `AppleInterfaceStyle`) lives
**once**, in `kanchi::probe`:

```rust
kanchi::defaxes! {
    /// Display-fit window size; falls back to 1200×800.
    window_dims: (u32, u32) = (1200, 800)
        => || kanchi::probe::screen_frac(0.60, (800, 600), (1600, 1100));
    /// DPR-aware font size — Retina 14pt / low-DPI 16pt.
    font_size: f32 = 14.0 => || kanchi::probe::dpr_font_size(14.0, 16.0);
    /// Scrollback scaled to physical RAM.
    scrollback_lines: u32 = 10_000
        => || kanchi::probe::total_ram_gib().map(|g| if g >= 16 { 100_000 } else { 10_000 });
    /// Not-yet-wired axis degenerates cleanly to the fallback.
    theme: &'static str = "nord" => kanchi::none;
}
```

Each line generates `FALLBACK_<NAME>` + `detect_<name>() -> Option<T>` +
`detect_<name>_or_fallback() -> T`. A probe is any no-arg callable returning
`Option<T>` (a `fn` path or a `|| …` closure baking in args); use
[`kanchi::none`] for axes whose detection isn't wired yet.

## probes (`kanchi::probe`)

| probe | source | `None` when |
|---|---|---|
| `screen_frac(frac, min, max)` | macOS NSScreen visible frame | off-main-thread / non-macOS |
| `dpr_font_size(hidpi, lodpi)` | macOS backing scale factor | off-main-thread / non-macOS |
| `appearance_dark()` | macOS `AppleInterfaceStyle` | non-macOS (xdg-portal/OSC11 TODO) |
| `total_ram_gib()` | `sysctl hw.memsize` / `/proc/meminfo` | unsupported OS |

## Why it exists

The prime-directive case: the identical trio appears ~7× in mado and is needed
in tear / escriba / namimado / ayatsuri / seki — ~126 hand-authored items the
fleet would otherwise re-vendor, each a place for drift, a `format!()`
violation, or a divergent platform-FFI path. `kanchi` makes the discovered
tier as cheap to author as `prescribed_default()` is, with fallbacks meant to
be sourced from the same `ishou::FleetDefaults` the prescribed tier reads.

mado is the reference consumer (`mado/src/auto_detect.rs`).

## License

MIT
