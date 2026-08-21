//! Window/surface tracking on top of the XDG shell.
//!
//! `XdgShellHandler` only tells us *that* a client asked for a window
//! or popup — it doesn't track *where* windows live or *what* is
//! currently mapped. That's what `smithay::desktop::Space` is for.
//!
//! This module is the bridge between the two: it wraps each new XDG
//! toplevel in a `Window` and keeps it in the compositor's `Space`,
//! which is exactly what the renderer (Stage 2) will iterate over to
//! know what exists and where to draw it.

use smithay::{
    desktop::{Space, Window},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::seat::WaylandFocus,
};

/// Finds the mapped `Window` whose toplevel surface is `surface`, if any.
pub fn window_for_surface(space: &Space<Window>, surface: &WlSurface) -> Option<Window> {
    space
        .elements()
        .find(|window| window.wl_surface().as_deref() == Some(surface))
        .cloned()
}

/// Picks a location for a newly-created window.
///
/// There's no real window manager yet (that's Stage 4), so this just
/// cascades new windows down and to the right a bit so they don't all
/// land in a single stack at (0, 0).
pub fn next_window_position(space: &Space<Window>) -> (i32, i32) {
    const STEP: i32 = 24;
    let n = space.elements().count() as i32;
    (n * STEP, n * STEP)
}
