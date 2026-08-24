//! GPU rendering for the MITOS desktop.
//!
//! Backend-agnostic rendering helpers for the MITOS compositor.
//!
//! Stage 3 responsibilities:
//! - desktop background
//! - translucent glass panels
//! - rounded corners
//! - panel borders
//! - panel highlights
//! - panel shadows
//! - client window composition
//!
//! Wayland itself does not provide the MITOS glass effect.
//! The visual shell is deliberately implemented here.

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
    utils::{Rectangle, Scale},
};

use crate::desktop::HomeScreenConfig;
use crate::theme::MitosTheme;

// ============================================================================
// GLASS PANEL
// ============================================================================

/// Description of a MITOS glass panel.
///
/// Geometry and visual properties live here. The renderer converts this
/// description into GPU render elements.
#[derive(Clone, Copy, Debug)]
pub struct GlassPanel {
    /// Top-left position in logical compositor coordinates.
    pub position: (i32, i32),

    /// Panel width and height in logical pixels.
    pub size: (i32, i32),

    /// Rounded-corner radius.
    pub radius: f32,

    /// Main translucent glass tint.
    pub tint: Color32F,

    /// Panel border color.
    pub border: Color32F,
}

impl GlassPanel {
    /// Create the MITOS top bar.
    pub fn top_bar(width: i32, height: i32) -> Self {
        Self::new(
            (0, 0),
            (width, height),
            MitosTheme::PANEL_RADIUS,
            glass_color(),
        )
    }

    /// Create the MITOS launcher.
    pub fn launcher(
        screen_width: i32,
        screen_height: i32,
    ) -> Self {
        let width = 720;
        let height = 480;

        let x = ((screen_width - width) / 2).max(0);
        let y = ((screen_height - height) / 2).max(0);

        Self::new(
            (x, y),
            (width, height),
            MitosTheme::PANEL_RADIUS,
            glass_color(),
        )
    }

    /// Create a generic glass panel.
    pub fn new(
        position: (i32, i32),
        size: (i32, i32),
        radius: f32,
        tint: Color32F,
    ) -> Self {
        let border = MitosTheme::BORDER;

        Self {
            position,
            size,
            radius,
            tint,
            border: Color32F::new(
                border.r,
                border.g,
                border.b,
                border.a,
            ),
        }
    }
}

// ============================================================================
// DESKTOP BACKGROUND
// ============================================================================

#[derive(Clone, Copy, Debug)]
pub enum BackgroundMode {
    Solid(Color32F),

    Gradient {
        top: Color32F,
        bottom: Color32F,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct DesktopBackground {
    pub mode: BackgroundMode,
}

impl DesktopBackground {
    pub fn from_home_screen(
        home_screen: &HomeScreenConfig,
    ) -> Self {
        let c = home_screen.background;

        Self {
            mode: BackgroundMode::Solid(
                Color32F::new(
                    c.r,
                    c.g,
                    c.b,
                    c.a,
                ),
            ),
        }
    }

    pub fn solid(color: Color32F) -> Self {
        Self {
            mode: BackgroundMode::Solid(color),
        }
    }

    pub fn gradient(
        top: Color32F,
        bottom: Color32F,
    ) -> Self {
        Self {
            mode: BackgroundMode::Gradient {
                top,
                bottom,
            },
        }
    }
}

pub fn background_color(
    home_screen: &HomeScreenConfig,
) -> Color32F {
    let c = home_screen.background;

    Color32F::new(
        c.r,
        c.g,
        c.b,
        c.a,
    )
}

pub fn clear_color(
    home_screen: &HomeScreenConfig,
) -> Color32F {
    background_color(home_screen)
}

// ============================================================================
// RENDER ELEMENT TYPES
// ============================================================================

render_elements! {
    pub ChromeRenderElement<=GlesRenderer>;

    Glass=PixelShaderElement,
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    SolidColor=SolidColorRenderElement,
}

// ============================================================================
// THEME COLORS
// ============================================================================

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

/// Subtle highlight used along the upper edge of glass panels.
pub fn glass_highlight_color() -> Color32F {
    let c = MitosTheme::GLASS_HIGHLIGHT;

    Color32F::new(
        c.r,
        c.g,
        c.b,
        c.a,
    )
}

/// Shadow used beneath glass panels.
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

// ============================================================================
// GLASS SHADER
// ============================================================================

/// Compile the reusable GPU shader used for MITOS glass panels.
///
/// The shader:
/// - draws a translucent panel
/// - creates rounded corners using a signed-distance function
/// - anti-aliases the rounded edge
///
/// Panel tint and radius are currently supplied when the shader is generated.
/// This keeps the implementation compatible with the current Smithay
/// `PixelShaderElement` API while we build the Stage 3 renderer.
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
    vec2 position = v_coords * size;

    vec2 half_size = size * 0.5;

    float radius = min(
        RADIUS,
        min(size.x, size.y) * 0.5
    );

    vec2 q = abs(position - half_size)
        - (half_size - vec2(radius));

    float distance = length(max(q, 0.0))
        + min(max(q.x, q.y), 0.0)
        - radius;

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

// ============================================================================
// GENERIC GLASS PANEL RENDERING
// ============================================================================

/// Render one glass panel and its supporting visual layers.
///
/// Every MITOS shell component uses the same rendering pipeline:
///
///     shadow
///        ↓
///     glass
///        ↓
///     highlight
///        ↓
///     border
///
/// This is the core of the MITOS Stage 3 visual shell.
fn collect_glass_panel_elements(
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

    if width <= 0 || height <= 0 {
        return elements;
    }

    // ------------------------------------------------------------
    // Rounded glass body
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
            glass_panel.clone(),
        ),
    );

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

// ============================================================================
// TOP BAR
// ============================================================================

pub fn collect_top_bar_elements(
    panel: &GlassPanel,
    glass_panel: &mut PixelShaderElement,
    shadow_buffer: &SolidColorBuffer,
    highlight_buffer: &SolidColorBuffer,
    border_buffer: &SolidColorBuffer,
    renderer: &mut GlesRenderer,
    scale: Scale<f64>,
) -> Vec<ChromeRenderElement> {
    collect_glass_panel_elements(
        panel,
        glass_panel,
        shadow_buffer,
        highlight_buffer,
        border_buffer,
        renderer,
        scale,
    )
}

// ============================================================================
// LAUNCHER
// ============================================================================

pub fn collect_launcher_elements(
    panel: &GlassPanel,
    glass_panel: &mut PixelShaderElement,
    shadow_buffer: &SolidColorBuffer,
    highlight_buffer: &SolidColorBuffer,
    border_buffer: &SolidColorBuffer,
    renderer: &mut GlesRenderer,
    scale: Scale<f64>,
) -> Vec<ChromeRenderElement> {
    collect_glass_panel_elements(
        panel,
        glass_panel,
        shadow_buffer,
        highlight_buffer,
        border_buffer,
        renderer,
        scale,
    )
}

// ============================================================================
// DOCK
// ============================================================================

pub fn collect_dock_elements(
    panel: &GlassPanel,
    glass_panel: &mut PixelShaderElement,
    shadow_buffer: &SolidColorBuffer,
    highlight_buffer: &SolidColorBuffer,
    border_buffer: &SolidColorBuffer,
    renderer: &mut GlesRenderer,
    scale: Scale<f64>,
) -> Vec<ChromeRenderElement> {
    collect_glass_panel_elements(
        panel,
        glass_panel,
        shadow_buffer,
        highlight_buffer,
        border_buffer,
        renderer,
        scale,
    )
}

// ============================================================================
// COMPLETE MITOS SHELL
// ============================================================================

/// Collect every visible MITOS shell element.
///
/// Shell order:
///
/// 1. Top bar
/// 2. Launcher, when visible
/// 3. Dock
pub fn collect_shell_elements(
    renderer: &mut GlesRenderer,
    shell: &crate::state::MitosShell,
    top_bar_glass: &mut PixelShaderElement,
    launcher_glass: &mut PixelShaderElement,
    dock_glass: &mut PixelShaderElement,
    top_bar_shadow: &SolidColorBuffer,
    top_bar_highlight: &SolidColorBuffer,
    top_bar_border: &SolidColorBuffer,
    scale: Scale<f64>,
) -> Vec<ChromeRenderElement> {
    let mut elements = Vec::new();

    // ------------------------------------------------------------
    // TOP BAR
    // ------------------------------------------------------------

    if let Some(panel) = shell.top_bar.as_ref() {
        elements.extend(
            collect_top_bar_elements(
                panel,
                top_bar_glass,
                top_bar_shadow,
                top_bar_highlight,
                top_bar_border,
                renderer,
                scale,
            ),
        );
    }

    // ------------------------------------------------------------
    // LAUNCHER
    // ------------------------------------------------------------

    if shell.launcher_visible {
        if let Some(panel) = shell.launcher.as_ref() {
            elements.extend(
                collect_launcher_elements(
                    panel,
                    launcher_glass,
                    top_bar_shadow,
                    top_bar_highlight,
                    top_bar_border,
                    renderer,
                    scale,
                ),
            );
        }
    }

    // ------------------------------------------------------------
    // DOCK
    // ------------------------------------------------------------

    if let Some(panel) = shell.dock.as_ref() {
        elements.extend(
            collect_dock_elements(
                panel,
                dock_glass,
                top_bar_shadow,
                top_bar_highlight,
                top_bar_border,
                renderer,
                scale,
            ),
        );
    }

    elements
}

// ============================================================================
// COMPLETE FRAME
// ============================================================================

/// Collect all render elements for one frame.
///
/// Shell elements are rendered first, followed by client windows.
///
/// Client windows are traversed from front to back according to the
/// compositor's `Space`.
pub fn collect_frame_elements(
    renderer: &mut GlesRenderer,
    space: &Space<Window>,
    scale: Scale<f64>,
    shell_elements: impl IntoIterator<Item = ChromeRenderElement>,
) -> Vec<ChromeRenderElement> {
    let mut elements = Vec::new();

    // ------------------------------------------------------------
    // MITOS shell
    // ------------------------------------------------------------

    elements.extend(shell_elements);

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
