//! `kanchi` (感知 — "sensing / perception") — a typed environment-discovery
//! primitive for the pleme-io fleet.
//!
//! Every GPU/terminal app faces the same domain: *probe the host for the
//! best-fit value, fall back to a documented default when it can't, expose
//! the result as a discoverable tier.* Today each app hand-rolls, per axis,
//! the identical trio:
//!
//! ```ignore
//! pub const FALLBACK_X: T = ...;
//! pub fn detect_x() -> Option<T> { /* probe */ }
//! pub fn detect_x_or_fallback() -> T { detect_x().unwrap_or(FALLBACK_X) }
//! ```
//!
//! That is the shikumi `discovered()` tier — but hand-written, so it drifts
//! (apps stub it out because it's too costly) and re-vendors subtle platform
//! FFI (NSScreen, sysctl, `/proc/meminfo`). `kanchi` collapses the whole
//! domain into a declaration:
//!
//! ```ignore
//! kanchi::defaxes! {
//!     /// Display-fit window size.
//!     window_dims: (u32, u32) = (1200, 800)
//!         => || kanchi::probe::screen_frac(0.60, (800, 600), (1600, 1100));
//!     /// DPR-aware font size.
//!     font_size: f32 = 14.0 => || kanchi::probe::dpr_font_size(14.0, 16.0);
//! }
//! ```
//!
//! Each line emits the full trio. The platform probes live once, in
//! [`probe`], cfg-gated and returning `None` off-platform so the resolver
//! lands cleanly on the fallback.

pub mod axes;
pub mod probe;

/// Re-export for the `defaxes!` macro's identifier-pasting. Not public API.
#[doc(hidden)]
pub use paste as __paste;

/// Probe placeholder for an axis whose detection isn't wired yet: always
/// `None`, so `detect_*_or_fallback()` returns the documented fallback.
/// Beats a hand-written stub — the axis still participates in the trio.
#[must_use]
pub fn none<T>() -> Option<T> {
    None
}

/// Clamp `v` into `[lo, hi]` (probes use it to keep detected values sane).
#[must_use]
pub fn clamp<T: PartialOrd>(v: T, lo: T, hi: T) -> T {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// Declare environment-detection axes. One line per axis generates a
/// documented `FALLBACK_<NAME>` const, a `detect_<name>() -> Option<T>`
/// probe wrapper, and a `detect_<name>_or_fallback() -> T` resolver.
///
/// Grammar (repeatable):
/// ```ignore
/// <#[doc…]> <name>: <Type> = <const-fallback> => <probe-expr>;
/// ```
/// `<probe-expr>` is anything callable with no args returning `Option<Type>`
/// — a `fn` path, or a `|| …` closure baking in arguments (e.g.
/// `|| kanchi::probe::dpr_font_size(14.0, 16.0)`). Use [`kanchi::none`](none)
/// for not-yet-wired axes.
#[macro_export]
macro_rules! defaxes {
    ($(
        $(#[$meta:meta])*
        $name:ident : $ty:ty = $fallback:expr => $probe:expr ;
    )+) => {
        $( $crate::__paste::paste! {
            $(#[$meta])*
            #[doc = concat!("\n\nSafe fallback for the `", stringify!($name), "` axis.")]
            pub const [<FALLBACK_ $name:upper>]: $ty = $fallback;

            $(#[$meta])*
            #[doc = concat!("\n\nBest-effort probe for `", stringify!($name), "`; `None` when unanswerable.")]
            #[must_use]
            pub fn [<detect_ $name>]() -> ::core::option::Option<$ty> {
                ($probe)()
            }

            #[doc = concat!("Resolve `", stringify!($name), "`: detection, else [`FALLBACK_", stringify!([<$name:upper>]), "`].")]
            #[must_use]
            pub fn [<detect_ $name _or_fallback>]() -> $ty {
                [<detect_ $name>]().unwrap_or([<FALLBACK_ $name:upper>])
            }
        } )+
    };
}

#[cfg(test)]
mod tests {
    // A detected axis + a not-yet-wired (fallback-only) axis.
    crate::defaxes! {
        /// Number of widgets.
        widget_count: u32 = 7 => || Some(42);
        /// Gizmos — detection not wired.
        gizmos: u32 = 3 => crate::none;
        /// A string axis to prove non-numeric types + const &str fallback.
        flavor: &'static str = "nord" => || Some("dracula");
    }

    #[test]
    fn detection_wins_when_some() {
        assert_eq!(detect_widget_count_or_fallback(), 42);
        assert_eq!(detect_flavor_or_fallback(), "dracula");
    }

    #[test]
    fn fallback_when_none() {
        assert!(detect_gizmos().is_none());
        assert_eq!(detect_gizmos_or_fallback(), 3);
    }

    #[test]
    fn fallback_consts_pinned() {
        assert_eq!(FALLBACK_WIDGET_COUNT, 7);
        assert_eq!(FALLBACK_GIZMOS, 3);
        assert_eq!(FALLBACK_FLAVOR, "nord");
    }

    #[test]
    fn clamp_bounds() {
        assert_eq!(crate::clamp(5, 0, 10), 5);
        assert_eq!(crate::clamp(-3, 0, 10), 0);
        assert_eq!(crate::clamp(99, 0, 10), 10);
    }
}
