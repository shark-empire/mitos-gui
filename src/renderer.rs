//! GPU rendering for the MITOS desktop.
//!
//! Backend-agnostic rendering helpers for the MITOS compositor.

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

/// Description of a MITOS glass panel.
///
/// This is currently shell-side geometry/style information.
/// The actual blur, rounded corners and compositing will be
/// implemented by the renderer in a later Stage 3 pass.
#[derive(Clone, Copy, Debug)]
pub struct GlassPanel {
    pub position: (i32, i32),
    pub size: (i32, i32),
    pub radius: f32,
    pub tint: Color32F,
    pub border: Color32F,
}

impl GlassPanel {
    /// Creates the standard MITOS top bar.
    pub fn top_bar(width: i32, height: i32) -> Self {
        let border = MitosTheme::BORDER;

        Self {
            position: (0, 0),
            size: (width, height),
            radius: MitosTheme::PANEL_RADIUS,
            tint: glass_color(),
            border: Color32F::new(
                border.r,
                border.g,
                border.b,
                border.a,
            ),
        }
    }
}

/// All actual renderable objects used by MITOS.
///
/// `GlassPanel` is intentionally NOT in this macro yet because it
/// is currently a style/geometry description rather than a Smithay
/// `RenderElement`.
render_elements! {
    pub ChromeRenderElement<=GlesRenderer>;

    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    SolidColor=SolidColorRenderElement,
}

/// Color used to clear the framebuffer.
///
/// The desktop wallpaper/background comes from `HomeScreenConfig`.
pub fn clear_color(home_screen: &HomeScreenConfig) -> Color32F {
    let c = home_screen.background;

    Color32F::new(
        c.r,
        c.g,
        c.b,
        c.a,
    )
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

/// Subtle highlight used for layered glass effects.
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

/// Color used by the MITOS top bar.
pub fn top_bar_color() -> Color32F {
    glass_color()
}

/// Build the visual layers of the MITOS top bar.
///
/// The current Stage 3 implementation intentionally uses ordinary
/// solid-color render elements. This gives us translucent glass,
/// highlight, border and shadow layers without pretending that
/// Wayland itself provides a glass effect.
///
/// Blur and rounded corners will be implemented later as an actual
/// renderer/shader pass.
pub fn collect_top_bar_elements(
    renderer: &mut GlesRenderer,
    panel: &GlassPanel,
    panel_buffer: &SolidColorBuffer,
    shadow_buffer: &SolidColorBuffer,
    highlight_buffer: &SolidColorBuffer,
    border_buffer: &SolidColorBuffer,
    scale: Scale<f64>,
) -> Vec<ChromeRenderElement> {
    let mut elements = Vec::new();

    let (x, y) = panel.position;
    let (width, height) = panel.size;

    // ------------------------------------------------------------
    // Shadow
    // ------------------------------------------------------------

    elements.extend(
        shadow_buffer.render_elements(
            renderer,
            (x, y + height).into(),
            scale,
            1.0,
        ),
    );

    // ------------------------------------------------------------
    // Main glass panel
    // ------------------------------------------------------------

    elements.extend(
        panel_buffer.render_elements(
            renderer,
            (x, y).into(),
            scale,
            1.0,
        ),
    );

    // ------------------------------------------------------------
    // Top highlight
    // ------------------------------------------------------------

    elements.extend(
        highlight_buffer.render_elements(
            renderer,
            (x, y).into(),
            scale,
            1.0,
        ),
    );

    // ------------------------------------------------------------
    // Bottom border
    // ------------------------------------------------------------

    elements.extend(
        border_buffer.render_elements(
            renderer,
            (x, y + height - 1).into(),
            scale,
            1.0,
        ),
    );

    elements
}

/// Collect render elements for one frame.
///
/// The MITOS shell is placed first, followed by client windows.
///
/// Windows are reversed so the frontmost window is processed first.
pub fn collect_frame_elements(
    renderer: &mut GlesRenderer,
    space: &Space<Window>,
    scale: Scale<f64>,
    top_bar_elements: impl IntoIterator<Item = ChromeRenderElement>,
) -> Vec<ChromeRenderElement> {
    let mut elements = Vec::new();

    // ------------------------------------------------------------
    // MITOS shell
    // ------------------------------------------------------------

    elements.extend(top_bar_elements);

    // ------------------------------------------------------------
    // Client windows
    // ------------------------------------------------------------

    for window in space.elements().rev() {
        let Some(location) = space.element_location(window) else {
            continue;
        };

        let physical_location = location
            .to_f64()
            .to_physical(scale)
            .to_i32_round();

        elements.extend(
            window.render_elements(
                renderer,
                physical_location,
                scale,
                1.0,
            ),
        );
    }

    elements
}
