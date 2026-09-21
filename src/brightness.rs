//! Stage 6: Best-effort screen brightness control.
//!
//! Mirrors `audio.rs`'s shape: try whatever brightness CLI is already on
//! `$PATH`, in order, and give up silently if none is available (e.g. a
//! VM with no backlight device). `brightnessctl` is the common
//! cross-desktop tool; `light` is a widely-packaged fallback.
//!
//! Fire-and-forget, same as `audio.rs` -- this never blocks the
//! compositor on a brightness daemon that isn't there.

use std::process::Command;

/// Set the primary display's brightness to `percent` (0-100).
pub fn set_brightness(percent: u8) {
    let pct = percent.min(100);

    if try_run("brightnessctl", &["set", &format!("{pct}%")]) {
        return;
    }

    let _ = try_run("light", &["-S", &pct.to_string()]);
}

fn try_run(bin: &str, args: &[&str]) -> bool {
    match Command::new(bin).args(args).status() {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}
