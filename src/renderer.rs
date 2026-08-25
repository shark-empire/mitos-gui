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
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{
                    MemoryRenderBuffer,
                    MemoryRenderBufferRenderElement,
                },
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
    },
    desktop::{Space, Window},
    utils::{Logical, Rectangle, Scale, Size, Transform},
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


// ============================================================================
// MITOS WALLPAPER
// ============================================================================

/// MITOS ships with its default wallpaper embedded into the executable.
///
/// This means the compositor does not depend on the current working
/// directory when it starts.
const DEFAULT_WALLPAPER: &[u8] =
    include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/wallpapers/mitos-default.png"
    ));

/// GPU-uploadable wallpaper source.
///
/// The image itself lives in a Smithay memory render buffer. Smithay
/// maintains the renderer-specific texture internally, so the PNG is
/// decoded once and the GPU texture is reused.
#[derive(Clone, Debug)]
pub struct Wallpaper {
    pub buffer: MemoryRenderBuffer,
    pub size: Size<i32, Logical>,
}

impl Wallpaper {
    /// Load the built-in MITOS wallpaper.
    pub fn load_default() -> Result<Self, String> {
        let image =
            image::load_from_memory(DEFAULT_WALLPAPER)
                .map_err(|err| {
                    format!(
                        "failed to decode MITOS wallpaper: {err}"
                    )
                })?;

        let rgba = image.to_rgba8();

        let width = rgba.width() as i32;
        let height = rgba.height() as i32;

        if width <= 0 || height <= 0 {
            return Err(
                "MITOS wallpaper has invalid dimensions"
                    .to_string()
            );
        }

        let buffer_size = (width, height).into();

let logical_size =
    Size::<i32, Logical>::new(width, height);

        let buffer =
            MemoryRenderBuffer::from_slice(
                rgba.as_raw(),
                Fourcc::Abgr8888,
                buffer_size,
                1,
                Transform::Normal,
                Some(vec![
                    Rectangle::from_size(buffer_size),
                ]),
            );

        println!(
            "MITOS GUI: wallpaper loaded ({}x{})",
            width,
            height
        );

        Ok(Self {
            buffer,
            size: logical_size,
        })
    }

    /// Create the render element for the current output.
    ///
    /// The image uses a "cover" strategy:
    ///
    /// - preserve aspect ratio
    /// - fill the entire screen
    /// - crop the excess
    pub fn render_element(
        &self,
        renderer: &mut GlesRenderer,
        output_size: Size<i32, Logical>,
    ) -> Result<
        MemoryRenderBufferRenderElement<GlesRenderer>,
        GlesError,
    > {
        let image_width =
            self.size.w as f64;

        let image_height =
            self.size.h as f64;

        let output_width =
            output_size.w.max(1) as f64;

        let output_height =
            output_size.h.max(1) as f64;

        let image_aspect =
            image_width / image_height;

        let output_aspect =
            output_width / output_height;

        let src = if image_aspect > output_aspect {
            // Image is wider than the screen.
            //
            // Crop left and right.
            let visible_width =
                image_height * output_aspect;

            let x =
                (image_width - visible_width) * 0.5;

            Rectangle::new(
                (x, 0.0).into(),
                (
                    visible_width,
                    image_height,
                )
                    .into(),
            )
        } else {
            // Image is taller than the screen.
            //
            // Crop top and bottom.
            let visible_height =
                image_width / output_aspect;

            let y =
                (image_height - visible_height) * 0.5;

            Rectangle::new(
                (0.0, y).into(),
                (
                    image_width,
                    visible_height,
                )
                    .into(),
            )
        };

        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            (0.0, 0.0),
            &self.buffer,
            Some(1.0),
            Some(src),
            Some(output_size),
            smithay::backend::renderer::element::Kind::Unspecified,
        )
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

    Wallpaper=MemoryRenderBufferRenderElement<GlesRenderer>,
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

pub fn collect_dock_elements(
    panel: &GlassPanel,
    layout: &crate::desktop::DockLayout,
    glass_panel: &mut PixelShaderElement,
    shadow_buffer: &SolidColorBuffer,
    highlight_buffer: &SolidColorBuffer,
    border_buffer: &SolidColorBuffer,
    renderer: &mut GlesRenderer,
    scale: Scale<f64>,
) -> Vec<ChromeRenderElement> {
    let mut elements = collect_glass_panel_elements(
        panel,
        glass_panel,
        shadow_buffer,
        highlight_buffer,
        border_buffer,
        renderer,
        scale,
    );

    // ------------------------------------------------------------
    // DOCK ICONS
    // ------------------------------------------------------------

    elements.extend(
        collect_dock_icon_elements(
            panel,
            layout,
            renderer,
            scale,
        ),
    );

    elements
}


// ============================================================================
// DOCK ICONS
// ============================================================================

fn collect_dock_icon_elements(
    panel: &GlassPanel,
    layout: &crate::desktop::DockLayout,
    renderer: &mut GlesRenderer,
    scale: Scale<f64>,
) -> Vec<ChromeRenderElement> {
    let mut elements = Vec::new();

    if layout.items.is_empty() {
        return elements;
    }

    let icon_size = layout.icon_size.max(1);
    let spacing = layout.spacing.max(0);

    let total_width =
        (layout.items.len() as i32 * icon_size)
        + ((layout.items.len().saturating_sub(1) as i32) * spacing);

    let start_x =
        panel.position.0
        + ((panel.size.0 - total_width) / 2).max(0);

    let center_y =
        panel.position.1
        + ((panel.size.1 - icon_size) / 2).max(0);

    for (index, item) in layout.items.iter().enumerate() {
        let x =
            start_x
            + index as i32 * (icon_size + spacing);

        let y = center_y;

        let color = if item.active {
            MitosTheme::ACCENT
        } else {
            MitosTheme::GLASS_LIGHT
        };

        let buffer = SolidColorBuffer::new(
            (icon_size, icon_size),
            Color32F::new(
                color.r,
                color.g,
                color.b,
                color.a,
            ),
        );

        elements.extend(
            buffer.render_elements(
                renderer,
                (x, y).into(),
                scale,
                1.0,
            ),
        );
    }

    elements
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
    dock_layout: &crate::desktop::DockLayout,

    top_bar_glass: &mut PixelShaderElement,
    launcher_glass: &mut PixelShaderElement,
    dock_glass: &mut PixelShaderElement,

    top_bar_shadow: &SolidColorBuffer,
    top_bar_highlight: &SolidColorBuffer,
    top_bar_border: &SolidColorBuffer,

    dock_shadow: &SolidColorBuffer,
    dock_highlight: &SolidColorBuffer,
    dock_border: &SolidColorBuffer,

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
          dock_layout,
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

pub fn collect_frame_elements(
    renderer: &mut GlesRenderer,
    space: &Space<Window>,
    scale: Scale<f64>,
    wallpaper: &Wallpaper,
    output_size: Size<i32, Logical>,
    shell_elements: impl IntoIterator<Item = ChromeRenderElement>,
    overlay_elements: impl IntoIterator<Item = ChromeRenderElement>,
) -> Result<Vec<ChromeRenderElement>, GlesError> {
    let mut elements = Vec::new();

    // ------------------------------------------------------------
    // 1. WALLPAPER
    // ------------------------------------------------------------

    let wallpaper_element =
        wallpaper.render_element(renderer, output_size)?;

    elements.push(
        ChromeRenderElement::Wallpaper(wallpaper_element)
    );

    // ------------------------------------------------------------
    // 2. MITOS SHELL
    //    Dock + top bar
    // ------------------------------------------------------------

    elements.extend(shell_elements);

    // ------------------------------------------------------------
    // 3. WAYLAND APPLICATION WINDOWS
    // ------------------------------------------------------------

    for window in space.elements().rev() {
        let Some(location) =
            space.element_location(window)
        else {
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

    // ------------------------------------------------------------
    // 4. MITOS OVERLAYS
    //    Launcher, dialogs, etc.
    // ------------------------------------------------------------

    elements.extend(overlay_elements);

    Ok(elements)
}
