//! GPU rendering for the MITOS desktop.
//!
//! Stage 2 wires an actual GPU renderer into the compositor:
//!
//!   - a real GLES context, via winit's EGL window (Stage 5 swaps this
//!     for a DRM/GBM swapchain, but the rendering code below doesn't
//!     change -- it only ever talks to `GlesRenderer`)
//!   - damage tracking, so a frame with nothing new to show costs
//!     (almost) nothing instead of repainting the whole output
//!   - frame scheduling, so mapped clients throttle their own redraws
//!     to this output's refresh rate instead of rendering as fast as
//!     they possibly can (see the frame-callback loop in main.rs)
//!
//! This module deliberately does *not* own the winit backend, the
//! `OutputDamageTracker`, or the event loop -- those live in `main.rs`,
//! where a single `render_output` call needs simultaneous access to the
//! backend, the damage tracker, and the compositor state, and splitting
//! that across a struct boundary just fights the borrow checker for no
//! real benefit. What lives here is the reusable, backend-agnostic
//! half: turning a `Space` into GL render elements, and picking the
//! color the framebuffer gets cleared to first.
//!
//! Stage 3 is what actually makes this interesting to look at -- glass
//! panels, blur, rounded corners, the wallpaper. For now every window
//! is just its own client-drawn pixels, floating on a flat background.

use smithay::{
    backend::renderer::{element::AsRenderElements, gles::GlesRenderer, Color32F},
    desktop::{Space, Window},
    utils::Scale,
};

pub use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;

/// The color the framebuffer is cleared to before anything else is
/// drawn -- i.e. what's visible through/behind every mapped window.
///
/// Stage 3 replaces this flat fill with the actual wallpaper; today
/// it's just `MitosTheme::BACKGROUND`, so an empty desktop still reads
/// as "MITOS" rather than a random GL clear color.
pub fn clear_color() -> Color32F {
    let c = crate::theme::MitosTheme::BACKGROUND;
    Color32F::new(c.r, c.g, c.b, c.a)
}

/// Collects render elements for every mapped window, front-to-back
/// (topmost window first).
///
/// That ordering matters: `OutputDamageTracker::render_output` uses
/// each element's opaque region to skip drawing whatever's fully
/// covered by the elements already processed, which only helps if the
/// frontmost (topmost, most likely to be covering something) elements
/// come first.
///
/// NOTE: `Space::elements()` is assumed here to yield elements
/// bottom-to-top (oldest/lowest window first, matching the usual "push
/// new things onto the top of the stack" convention) -- this list
/// reverses that with `.rev()` to get the front-to-back order the
/// renderer wants. If windows come out stacked in the wrong order
/// against a real build, this assumption is the first thing to check.
pub fn collect_window_elements(
    renderer: &mut GlesRenderer,
    space: &Space<Window>,
    scale: Scale<f64>,
) -> Vec<WaylandSurfaceRenderElement<GlesRenderer>> {
    let mut elements = Vec::new();

    for window in space.elements().rev() {
        let Some(location) = space.element_location(window) else {
            continue;
        };

        // Logical -> physical, rounded to whole pixels -- the same
        // conversion chain used for cursor placement, kept consistent
        // here for anything that mixes window and cursor coordinates
        // later (e.g. Stage 4's resize handles).
        let physical_location = location.to_f64().to_physical(scale).to_i32_round();

        // `Window` implements `AsRenderElements`, which walks the
        // window's whole surface tree (subsurfaces included) for us --
        // no need to fetch `wl_surface()` and walk it by hand.
        elements.extend(window.render_elements(renderer, physical_location, scale, 1.0));
    }

    elements
}
