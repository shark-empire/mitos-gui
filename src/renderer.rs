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
        gles::{
            element::PixelShaderElement,
            GlesError,
            GlesRenderer,
        },
        Color32F,
    },
    desktop::{Space, Window},
    utils::{Logical, Rectangle, Scale},
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

render_elements! {
    pub ChromeRenderElement<=GlesRenderer>;

    Glass=PixelShaderElement,
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

/// Compile the GPU shader used for the MITOS glass panel.
///
/// The shader generates the rounded corners directly in GLES.
/// The panel geometry is supplied later through `PixelShaderElement::resize`.
pub fn create_glass_panel_element(
    renderer: &mut GlesRenderer,
) -> Result<PixelShaderElement, GlesError> {
    let glass = MitosTheme::GLASS;

    let shader = format!(
        r#"
precision mediump float;

varying vec2 v_coords;

uniform vec2 size;

const float RADIUS = {radius};

const vec4 GLASS_COLOR = vec4(
    {r},
    {g},
    {b},
    {a}
);

void main() {{
    // Convert normalized coordinates into panel pixels.
    vec2 position = v_coords * size;

    vec2 half_size = size * 0.5;

    // Prevent the radius from becoming larger than half
    // the smallest panel dimension.
    float radius = min(
        RADIUS,
        min(size.x, size.y) * 0.5
    );

    // Signed-distance rounded rectangle.
    vec2 q = abs(position - half_size)
        - (half_size - vec2(radius));

    float distance = length(max(q, 0.0))
        + min(max(q.x, q.y), 0.0)
        - radius;

    // One-pixel anti-aliased edge.
    float alpha = 1.0 - smoothstep(
        0.0,
        1.0,
        distance
    );

    gl_FragColor = vec4(
        GLASS_COLOR.rgb,
        GLASS_COLOR.a * alpha
    );
}}
"#,
        radius = MitosTheme::PANEL_RADIUS,
        r = glass.r,
        g = glass.g,
        b = glass.b,
        a = glass.a,
    );

    let program = renderer.compile_custom_pixel_shader(
        shader,
        &[],
    )?;

    Ok(PixelShaderElement::new(
        program,
        Rectangle::new(
            (0, 0).into(),
            (1, 1).into(),
        ),
        None,
        1.0,
        Vec::new(),
        smithay::backend::renderer::element::Kind::Unspecified,
    ))
}

pub fn collect_top_bar_elements(
    panel: &GlassPanel,
    glass_panel: &mut PixelShaderElement,
    shadow_buffer: &SolidColorBuffer,
    highlight_buffer: &SolidColorBuffer,
    border_buffer: &SolidColorBuffer,
    renderer: &mut GlesRenderer,
    scale: Scale<f64>,
) -> Vec<ChromeRenderElement> {
    let mut elements = Vec::new();

    let (x, y) = panel.position;
    let (width, height) = panel.size;

    // ------------------------------------------------------------
    // Rounded GPU glass panel
    // ------------------------------------------------------------

    glass_panel.resize(
        Rectangle::new(
            (x, y).into(),
            (width, height).into(),
        ),
        None,
    );

    elements.push(
        ChromeRenderElement::Glass(
            glass_panel.clone()
        )
    );

    // ------------------------------------------------------------
    // Soft shadow
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
    // Highlight
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
