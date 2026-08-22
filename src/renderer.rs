//! GPU rendering for the MITOS desktop.
//!
//! Stage 2 wired an actual GPU renderer into the compositor:
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
//! half: turning a `Space` (plus whatever chrome MITOS draws itself)
//! into GL render elements, and picking the colors involved.
//!
//! Stage 3 is what actually makes this interesting to look at -- glass
//! panels, blur, rounded corners, the top bar. Wallpaper customization
//! (`clear_color`, driven by `desktop::HomeScreenConfig`) was the first
//! piece. The top bar is the second: a flat, translucent panel drawn
//! with `ChromeRenderElement::SolidColor`. Blur and rounded corners
//! stay follow-up work -- blur in particular needs a second render
//! pass (sample the framebuffer behind the panel, then composite),
//! which is a bigger change than "draw one more rectangle" and isn't
//! worth taking on in the same step as wiring up the element type.
//! Every *window*, meanwhile, is still just its own client-drawn
//! pixels floating on top of everything else.

use smithay::{
    backend::renderer::{
        element::{
            render_elements,
            solid::{SolidColorBuffer, SolidColorRenderElement},
            surface::WaylandSurfaceRenderElement,
            AsRenderElements,
        },
        gles::GlesRenderer,
        Color32F,
    },
    desktop::{Space, Window},
    utils::Scale,
};

use crate::desktop::HomeScreenConfig;
use crate::theme::MitosTheme;

render_elements! {
    /// All renderable objects used by MITOS.
    pub ChromeRenderElement<=GlesRenderer>;

    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    SolidColor=SolidColorRenderElement,
}

/// The color the framebuffer is cleared to before anything else is
/// drawn -- i.e. what's visible through/behind every mapped window.
///
/// Reads from `HomeScreenConfig` rather than `MitosTheme::BACKGROUND`
/// directly, so the empty desktop reflects whatever the user set in
/// `~/.config/mitos/home.conf` instead of always being the built-in
/// default.
pub fn clear_color(home_screen: &HomeScreenConfig) -> Color32F {
    let c = home_screen.background;
    Color32F::new(c.r, c.g, c.b, c.a)
}

/// Main translucent MITOS glass color.
pub fn glass_color() -> Color32F {
    let c = MitosTheme::GLASS;

    Color32F::new(
        c.r,
        c.g,
        c.b,
        c.a,
    )
}

/// Subtle highlight used to give the panel a layered appearance.
pub fn glass_highlight_color() -> Color32F {
    let c = MitosTheme::GLASS_HIGHLIGHT;

    Color32F::new(
        c.r,
        c.g,
        c.b,
        c.a,
    )
}

/// Shadow color used behind shell panels.
pub fn shadow_color() -> Color32F {
    let c = MitosTheme::SHADOW;

    Color32F::new(
        c.r,
        c.g,
        c.b,
        c.a,
    )
}

/// The glass tint chrome panels are drawn with -- the top bar today.
///
/// Unlike `clear_color`, this doesn't read `HomeScreenConfig`: the
/// wallpaper is a "make it yours" setting, but the glass tint is part
/// of what makes MITOS chrome look like MITOS chrome, so for now it
/// stays a theme constant rather than something `home.conf` can
/// override. `home.conf` only controls whether the bar is drawn at
/// all and how tall it is (`top_bar` / `top_bar_height`).
pub fn top_bar_color() -> Color32F {
    glass_color()
}

/// Collects render elements for one frame: the top bar first (if
/// enabled), then every mapped window, both front-to-back (topmost
/// element first).
///
/// That ordering matters: `OutputDamageTracker::render_output` uses
/// each element's opaque region to skip drawing whatever's fully
/// covered by the elements already processed, which only helps if the
/// frontmost (topmost, most likely to be covering something) elements
/// come first. The top bar is meant to sit above every window, so it
/// leads the list even though nothing yet stops a window from being
/// dragged over it -- that enforcement is Stage 4's job.
///
/// `top_bar` takes the buffer rather than owning one: a `SolidColorBuffer`
/// only skips redundant work (and redundant damage) if the *same*
/// buffer is reused and updated frame to frame, so it has to live
/// somewhere that survives across calls -- that's `main.rs`, next to
/// `damage_tracker` and the other per-frame render state.
///
/// NOTE: `Space::elements()` is assumed here to yield elements
/// bottom-to-top (oldest/lowest window first, matching the usual "push
/// new things onto the top of the stack" convention) -- this list
/// reverses that with `.rev()` to get the front-to-back order the
/// renderer wants. If windows come out stacked in the wrong order
/// against a real build, this assumption is the first thing to check.
pub fn collect_frame_elements(
    renderer: &mut GlesRenderer,
    space: &Space<Window>,
    scale: Scale<f64>,
    top_bar: Option<&SolidColorBuffer>,
) -> Vec<ChromeRenderElement> {
    let mut elements = Vec::new();

    // ------------------------------------------------------------
    // MITOS shell
    // ------------------------------------------------------------

    if let Some(top_bar) = top_bar {
        elements.extend(
            top_bar.render_elements(
                renderer,
                (0, 0).into(),
                scale,
                1.0,
            )
        );
    }

    // ------------------------------------------------------------
    // Client windows
    // ------------------------------------------------------------

    for window in space.elements().rev() {
        let Some(location) = space.element_location(window) else {
            continue;
        };

        let physical_location =
            location
                .to_f64()
                .to_physical(scale)
                .to_i32_round();

        elements.extend(
            window.render_elements(
                renderer,
                physical_location,
                scale,
                1.0,
            )
        );
    }

    elements
}
