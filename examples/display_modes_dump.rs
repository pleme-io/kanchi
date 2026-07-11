//! Throwaway diagnostic: dump the full CoreGraphics display-mode table for the
//! main display, plus the CGDisplay bounds / pixel dimensions, so we can see
//! exactly what `display_scaling_ratio()` has to work with on this machine.
//!
//! Run natively on the target Mac:
//!   cargo run --example display_modes_dump
//!
//! Not built off-macOS.

#[cfg(target_os = "macos")]
fn main() {
    use objc2_core_foundation::{CFArrayGetCount, CFArrayGetValueAtIndex};
    use objc2_core_graphics::{
        CGDisplayBounds, CGDisplayCopyAllDisplayModes, CGDisplayCopyDisplayMode, CGDisplayMode,
        CGDisplayModeGetHeight, CGDisplayModeGetIOFlags, CGDisplayModeGetPixelHeight,
        CGDisplayModeGetPixelWidth, CGDisplayModeGetRefreshRate, CGDisplayModeGetWidth,
        CGDisplayPixelsHigh, CGDisplayPixelsWide, CGMainDisplayID,
    };

    const NATIVE_MODE_FLAG: u32 = 0x0200_0000;

    let display = CGMainDisplayID();

    let bounds = CGDisplayBounds(display);
    let px_wide = CGDisplayPixelsWide(display);
    let px_high = CGDisplayPixelsHigh(display);

    println!("=== CGDisplay main ({display}) ===");
    println!(
        "CGDisplayBounds (points): {}x{} at ({}, {})",
        bounds.size.width, bounds.size.height, bounds.origin.x, bounds.origin.y
    );
    println!("CGDisplayPixelsWide/High (current fb px): {px_wide}x{px_high}");

    let current = CGDisplayCopyDisplayMode(display);
    if let Some(cur) = current.as_ref() {
        println!(
            "CURRENT mode: point {}x{}  pixel {}x{}  ioflags 0x{:08x}  native={}",
            CGDisplayModeGetWidth(Some(cur)),
            CGDisplayModeGetHeight(Some(cur)),
            CGDisplayModeGetPixelWidth(Some(cur)),
            CGDisplayModeGetPixelHeight(Some(cur)),
            CGDisplayModeGetIOFlags(Some(cur)),
            CGDisplayModeGetIOFlags(Some(cur)) & NATIVE_MODE_FLAG != 0,
        );
    }

    // Production uses the default mode list (options = None).
    let modes = unsafe { CGDisplayCopyAllDisplayModes(display, None) };
    let Some(modes) = modes else {
        println!("CGDisplayCopyAllDisplayModes returned None");
        return;
    };
    let count = unsafe { CFArrayGetCount(&modes) };
    println!("\n=== {count} display modes (default list; idx: point | pixel | flags | native) ===");

    let mut native_pixels = Vec::new();
    let mut all_pixel_heights = Vec::new();
    for i in 0..count {
        let ptr = unsafe { CFArrayGetValueAtIndex(&modes, i) }.cast::<CGDisplayMode>();
        if ptr.is_null() {
            continue;
        }
        let mode = unsafe { &*ptr };
        let pw = CGDisplayModeGetWidth(Some(mode));
        let ph = CGDisplayModeGetHeight(Some(mode));
        let pxw = CGDisplayModeGetPixelWidth(Some(mode));
        let pxh = CGDisplayModeGetPixelHeight(Some(mode));
        let flags = CGDisplayModeGetIOFlags(Some(mode));
        let native = flags & NATIVE_MODE_FLAG != 0;
        let refresh = CGDisplayModeGetRefreshRate(Some(mode));
        all_pixel_heights.push(pxh);
        if native {
            native_pixels.push((pxw, pxh));
        }
        println!(
            "{i:3}: point {pw:>5}x{ph:<5} | pixel {pxw:>5}x{pxh:<5} | flags 0x{flags:08x} | native={native} | {refresh}Hz"
        );
    }

    println!("\n=== SUMMARY ===");
    println!("native-flagged modes (pixel wxh): {native_pixels:?}");
    let max_native_h = native_pixels.iter().map(|(_, h)| *h).max().unwrap_or(0);
    let max_any_h = all_pixel_heights.iter().copied().max().unwrap_or(0);
    println!("max native-flagged pixel height: {max_native_h}");
    println!("max any pixel height: {max_any_h}");
    if let Some(cur) = current.as_ref() {
        let fb_h = CGDisplayModeGetPixelHeight(Some(cur));
        println!("current fb pixel height (fb_h): {fb_h}");
        if fb_h > 0 && max_native_h > 0 {
            println!(
                "OLD ratio (max_native_h / fb_h) = {:.4}",
                max_native_h as f64 / fb_h as f64
            );
        }
    }
    println!(
        "\nexpected correct ratio on the XDR panel = 2234/2658 = {:.4}",
        2234.0 / 2658.0
    );
    println!("also: live probe display_scaling_ratio() = {:?}", kanchi::probe::display_scaling_ratio());
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("macOS only");
}
