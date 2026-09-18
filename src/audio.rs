//! Best-effort system volume control via whatever audio CLI is already
//! on `$PATH` -- `wpctl` (PipeWire/WirePlumber, the modern default on
//! most current distros) first, falling back to `pactl` (PulseAudio, or
//! PipeWire's pulse-compatible shim) if that's not installed. Same
//! try-in-order-and-fall-back shape `shell_interaction.rs` already uses
//! for launching apps.
//!
//! Deliberately NOT a real mixer integration (no device enumeration, no
//! per-app volume, no libpipewire/libpulse bindings) -- that's a much
//! bigger, separate piece of work. This is the smallest thing that
//! makes the existing volume keys/OSD (`keyboard.rs`, `state.rs`)
//! actually move a real fader instead of just an internal number, with
//! zero new Cargo dependencies.
//!
//! Every call here is fire-and-forget: on a system with neither tool
//! installed (or no audio server running at all), these are silent
//! no-ops -- the OSD and `state.volume`/`state.muted` still update
//! either way, so the UI never looks broken, it just isn't backed by
//! anything real in that case.

use std::process::Command;

/// Set the default output's volume to `percent` (0-100).
pub fn set_volume(percent: u8) {
    let pct = percent.min(100);

    if try_run("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{pct}%")]) {
        return;
    }

    let _ = try_run("pactl", &["set-sink-volume", "@DEFAULT_SINK@", &format!("{pct}%")]);
}

/// Set the default output's mute state.
pub fn set_muted(muted: bool) {
    if try_run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", if muted { "1" } else { "0" }]) {
        return;
    }

    let _ = try_run("pactl", &["set-sink-mute", "@DEFAULT_SINK@", if muted { "1" } else { "0" }]);
}

/// Runs `bin args...`, returning whether it was found *and* exited
/// successfully -- either failure means "try the next tool in the
/// fallback chain", so callers don't need to distinguish "not
/// installed" from "ran and errored".
fn try_run(bin: &str, args: &[&str]) -> bool {
    match Command::new(bin).args(args).status() {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}
