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
pub mod frosted_glass;

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
                Kind,
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
    utils::{Logical, Point, Rectangle, Scale, Size, Transform},
};

use crate::desktop::HomeScreenConfig;
use crate::theme::MitosTheme;


/// Captures the current scene (wallpaper + windows) into an offscreen texture.
pub fn capture_background(
    renderer: &mut GlesRenderer,
    output_size: Size<i32, Physical>,
    elements: &[impl Element<GlesRenderer>],
) -> Result<GlesTexture, Box<dyn std::error::Error>> {
    // 1. Create offscreen buffer
    let mut bg_texture = renderer.create_buffer(Fourcc::Abgr8888, output_size)?;
    let mut target = renderer.bind(&mut bg_texture)?;
    
    // 2. Render elements to the offscreen target
    // We use a simple damage tracker or just render everything
    let mut tracker = smithay::backend::renderer::damage::OutputDamageTracker::new(
        output_size, 1.0, Transform::Normal
    );
    
    smithay::backend::renderer::damage::render_output(
        renderer,
        &mut target,
        &mut tracker,
        0,
        [0.0, 0.0, 0.0, 1.0].into(), // Clear color
        elements.iter(),
    )?;

    // 3. Unbind
    drop(target);
    
    Ok(bg_texture)
}


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
            MitosTheme::effective_panel_radius(),
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
            MitosTheme::effective_panel_radius(),
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
    fn from_rgba(rgba: image::RgbaImage) -> Result<Self, String> {
        let width = rgba.width() as i32;
        let height = rgba.height() as i32;

        if width <= 0 || height <= 0 {
            return Err("wallpaper has invalid dimensions".to_string());
        }

        let buffer_size = (width, height).into();

        let buffer = MemoryRenderBuffer::from_slice(
            rgba.as_raw(),
            Fourcc::Abgr8888,
            buffer_size,
            1,
            Transform::Normal,
            Some(vec![Rectangle::from_size(buffer_size)]),
        );

        Ok(Self {
            buffer,
            size: Size::<i32, Logical>::new(width, height),
        })
    }

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

        println!(
            "MITOS GUI: wallpaper loaded ({}x{})",
            rgba.width(),
            rgba.height()
        );

        Self::from_rgba(rgba)
    }

    /// Load a wallpaper from disk (set via home.conf `wallpaper = ...`).
    pub fn load_from_path(path: &str) -> Result<Self, String> {
        let data = std::fs::read(path)
            .map_err(|err| format!("failed to read wallpaper {path}: {err}"))?;

        let image = image::load_from_memory(&data)
            .map_err(|err| format!("failed to decode wallpaper {path}: {err}"))?;

        println!("MITOS GUI: wallpaper loaded from {path}");

        Self::from_rgba(image.to_rgba8())
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
            Kind::Unspecified,
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
    Text=MemoryRenderBufferRenderElement<GlesRenderer>,
    Glass=PixelShaderElement,
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    SolidColor=SolidColorRenderElement,
}


// ============================================================================
// THEME COLORS
// ============================================================================

/// Main translucent MITOS glass color.
pub fn glass_color() -> Color32F {
    let c = MitosTheme::effective_glass();

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

/// Compile the reusable GPU shader used for MITOS liquid glass panels.
///
/// The shader layers, in order:
///   1. rounded SDF mask with anti-aliased edge
///   2. translucent tint
///   3. fresnel rim light (bright edge ring)
///   4. top specular sweep (light source above)
///   5. diagonal liquid sheen
///   6. chromatic refraction tint shift near edges
///   7. fine surface grain
pub fn create_glass_panel_element(
    renderer: &mut GlesRenderer,
) -> Result<PixelShaderElement, GlesError> {
    let glass = MitosTheme::effective_glass();
    let radius = MitosTheme::effective_panel_radius();

    let shader = format!(
        r#"
precision mediump float;

varying vec2 v_coords;
uniform vec2 size;

const float RADIUS = {radius:.8};

const vec4 TINT = vec4(
    {r:.8},
    {g:.8},
    {b:.8},
    {a:.8}
);

const float SPECULAR = {specular:.8};
const float RIM      = {rim:.8};
const float GRAIN    = {grain:.8};

float sd_round_box(vec2 p, vec2 half_size, float r) {{
    vec2 q = abs(p) - half_size + vec2(r);
    return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r;
}}

float hash(vec2 p) {{
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453123);
}}

void main() {{
    vec2 p = v_coords * size;
    vec2 half_size = size * 0.5;

    // ------------------------------------------------
    // Rounded mask (anti-aliased)
    // ------------------------------------------------
    float d = sd_round_box(p - half_size, half_size, RADIUS);
    float mask = 1.0 - smoothstep(-1.0, 1.0, d);

    if (mask <= 0.001) {{
        gl_FragColor = vec4(0.0);
        return;
    }}

    // ------------------------------------------------
    // Edge distance: 0 deep inside → 1 at the rim
    // ------------------------------------------------
    float edge = smoothstep(-8.0, 0.0, d);
    float inner = 1.0 - edge;

    // ------------------------------------------------
    // Light model
    // ------------------------------------------------
    // Top specular sweep (light from above)
    float top_light = smoothstep(0.15, 0.9, 1.0 - v_coords.y);

    // Diagonal liquid sheen
    float sheen =
        (sin((v_coords.x + v_coords.y * 0.7) * 6.28318) * 0.5 + 0.5);
    sheen = smoothstep(0.6, 1.0, sheen) * 0.06;

    // Fine grain so the surface reads as real material
    float grain = (hash(p) - 0.5) * GRAIN;

    // ------------------------------------------------
    // Compose color
    // ------------------------------------------------
    vec3 color = TINT.rgb;

    // Chromatic refraction shift at the rim (liquid look)
    color.r += edge * 0.04;
    color.g += edge * 0.06;
    color.b += edge * 0.10;

    // Specular + sheen + rim + grain
    color += top_light * SPECULAR * 0.30;
    color += sheen;
    color += edge * inner * RIM * 0.35;
    color += grain;

    // ------------------------------------------------
    // Alpha
    // ------------------------------------------------
    float alpha = TINT.a * mask;
    // Rim light ring just inside the edge
    alpha = max(alpha, edge * inner * RIM * 0.45 * mask);

    gl_FragColor = vec4(color, alpha);
}}
"#,
        radius = radius,
        r = glass.r,
        g = glass.g,
        b = glass.b,
        a = glass.a,
        specular = MitosTheme::effective_specular(),
        rim = MitosTheme::LIQUID_RIM,
        grain = MitosTheme::LIQUID_GRAIN,
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
        Kind::Unspecified,
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
    pointer_x: f64,
) -> Vec<ChromeRenderElement> {
    let mut elements = collect_glass_panel_elements(
        panel, glass_panel, shadow_buffer, highlight_buffer,
        border_buffer, renderer, scale,
    );

    elements.extend(collect_dock_icon_elements(
        panel, layout, renderer, scale, pointer_x,
    ));

    elements
}

fn collect_dock_icon_elements(
    panel: &GlassPanel,
    layout: &crate::desktop::DockLayout,
    renderer: &mut GlesRenderer,
    scale: Scale<f64>,
    pointer_x: f64,
) -> Vec<ChromeRenderElement> {
    let mut elements = Vec::new();

    if layout.items.is_empty() {
        return elements;
    }

    let icon_size = layout.icon_size.max(1) as f32;
    let spacing = layout.spacing.max(0) as f32;
    let sigma = icon_size * 2.2;
    let max_mag = MitosTheme::DOCK_MAGNIFICATION;

    // ------------------------------------------------
    // Gaussian magnification around the pointer
    // ------------------------------------------------
    let scales: Vec<f32> = layout
        .items
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let center_x =
                panel.position.0 as f32 + i as f32 * (icon_size + spacing) + icon_size * 0.5;
            let dist = pointer_x as f32 - center_x;
            let influence = (-(dist * dist) / (2.0 * sigma * sigma)).exp();
            1.0 + max_mag * influence
        })
        .collect();

    // Base (non-magnified) centered layout
    let total_width = (layout.items.len() as f32 * icon_size)
        + ((layout.items.len().saturating_sub(1)) as f32 * spacing);

    let start_x = panel.position.0 as f32
        + ((panel.size.0 as f32 - total_width) / 2.0).max(0.0);

    // Icons grow upward from a baseline near the dock bottom
    let baseline = (panel.position.1 + panel.size.1 - 8) as f32;

    for (index, item) in layout.items.iter().enumerate() {
        let s = scales[index];
        let size_i = (icon_size * s) as i32;

        let x = (start_x
            + index as f32 * (icon_size + spacing)
            + icon_size * (1.0 - s) * 0.5) as i32;

        let y = (baseline - size_i as f32) as i32;

        let color = if item.active {
            MitosTheme::effective_accent()
        } else {
            MitosTheme::GLASS_LIGHT
        };

        let buffer = SolidColorBuffer::new(
            (size_i, size_i),
            Color32F::new(color.r, color.g, color.b, color.a),
        );

        elements.extend(buffer.render_elements(
            renderer,
            (x, y).into(),
            scale,
            1.0,
        ));
    }

    elements
}


// ============================================================================
// SHELL TEXT STATE (clock + launcher search)
// ============================================================================

/// Caches rasterized shell text; re-rasterizes only when strings change.
pub struct ShellTextState {
    text_renderer: crate::text::TextRenderer,

    clock_string: String,
    clock_texture: Option<crate::text::TextTexture>,

    query_string: String,
    query_texture: Option<crate::text::TextTexture>,

    result_names: Vec<String>,
    result_textures: Vec<Option<crate::text::TextTexture>>,
}

impl ShellTextState {
    pub fn new() -> Self {
        Self {
            text_renderer: crate::text::TextRenderer::new(),
            clock_string: String::new(),
            clock_texture: None,
            query_string: String::new(),
            query_texture: None,
            result_names: Vec::new(),
            result_textures: Vec::new(),
        }
    }

    /// Re-rasterize any text that changed. Returns true if anything
    /// changed, so the caller can request a redraw.
    pub fn refresh(&mut self, shell: &crate::state::MitosShell) -> bool {
        let mut changed = false;

        // --------------------------------------------------------
        // Top bar clock (changes once per minute)
        // --------------------------------------------------------
        let now = crate::shell_interaction::current_time_string();

        if now != self.clock_string {
            self.clock_string = now.clone();
            self.clock_texture = self
                .text_renderer
                .render(&now, 14.0, (235, 240, 250, 255))
                .and_then(crate::text::TextTexture::from_rgba);
            changed = true;
        }

        // --------------------------------------------------------
        // Launcher search text
        // --------------------------------------------------------
        if shell.launcher_visible {
            let q = if shell.launcher_query.is_empty() {
                "Type to search".to_string()
            } else {
                shell.launcher_query.clone()
            };

            if q != self.query_string {
                self.query_string = q.clone();
                self.query_texture = self
                    .text_renderer
                    .render(&q, 20.0, (255, 255, 255, 255))
                    .and_then(crate::text::TextTexture::from_rgba);
                changed = true;
            }

            let names: Vec<String> = shell
                .launcher_results
                .iter()
                .take(8)
                .map(|a| a.name.clone())
                .collect();

            if names != self.result_names {
                self.result_names = names.clone();
                self.result_textures = names
                    .iter()
                    .map(|n| {
                        self.text_renderer
                            .render(n, 16.0, (220, 226, 238, 255))
                            .and_then(crate::text::TextTexture::from_rgba)
                    })
                    .collect();
                changed = true;
            }
        }

        changed
    }
}

// ============================================================================
// SYSTEM TRAY STATE (STAGE 6)
// ============================================================================

const TRAY_COLOR: (u8, u8, u8, u8) = (228, 233, 244, 255);

/// Caches rasterized tray icons; re-rasterizes only on status change.
pub struct TrayState {
    net_key: (u8, u8),
    net_tex: Option<crate::text::TextTexture>,

    vol_key: (u8, bool),
    vol_tex: Option<crate::text::TextTexture>,

    bat_key: (u8, u8, bool),
    bat_tex: Option<crate::text::TextTexture>,
}

impl TrayState {
    pub fn new() -> Self {
        Self {
            net_key: (255, 255),
            net_tex: None,
            vol_key: (255, false),
            vol_tex: None,
            bat_key: (255, 0, false),
            bat_tex: None,
        }
    }

    pub fn refresh(
        &mut self,
        network: &crate::status::NetworkStatus,
        battery: &Option<crate::status::BatteryStatus>,
        volume: u8,
        muted: bool,
    ) -> bool {
        use crate::status::NetworkStatus;

        let mut changed = false;

        let nk = match network {
            NetworkStatus::Offline => (0u8, 0u8),
            NetworkStatus::Ethernet => (1, 0),
            NetworkStatus::Wifi(l) => (2, *l),
        };

        if nk != self.net_key {
            self.net_key = nk;

            let img = match network {
                NetworkStatus::Offline =>
                    crate::icons::wifi_icon(18, 0, (150, 155, 165, 255)),
                NetworkStatus::Ethernet =>
                    crate::icons::ethernet_icon(18, TRAY_COLOR),
                NetworkStatus::Wifi(l) =>
                    crate::icons::wifi_icon(18, 1 + l / 34, TRAY_COLOR),
            };

            self.net_tex = crate::text::TextTexture::from_rgba(img);
            changed = true;
        }

        let vk = (volume, muted);

        if vk != self.vol_key {
            self.vol_key = vk;
            self.vol_tex = crate::text::TextTexture::from_rgba(
                crate::icons::volume_icon(18, volume, muted, TRAY_COLOR),
            );
            changed = true;
        }

        let bk = match battery {
            Some(b) => (1u8, b.capacity, b.charging),
            None => (0, 0, false),
        };

        if bk != self.bat_key {
            self.bat_key = bk;
            self.bat_tex = battery.map(|b| {
                crate::text::TextTexture::from_rgba(
                    crate::icons::battery_icon(b.capacity, b.charging, TRAY_COLOR),
                )
            });
            changed = true;
        }

        changed
    }

    pub fn total_width(&self) -> i32 {
        let mut w = 0;
        let mut n = 0;

        for t in [&self.net_tex, &self.vol_tex, &self.bat_tex]
            .into_iter()
            .flatten()
        {
            w += t.size.w;
            n += 1;
        }

        if n > 0 { w + (n - 1) * 10 } else { 0 }
    }
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
    pointer: (f64, f64),

    top_bar_glass: &mut PixelShaderElement,
    launcher_glass: &mut PixelShaderElement,
    dock_glass: &mut PixelShaderElement,

    top_bar_shadow: &SolidColorBuffer,
    top_bar_highlight: &SolidColorBuffer,
    top_bar_border: &SolidColorBuffer,

    dock_shadow: &SolidColorBuffer,
    dock_highlight: &SolidColorBuffer,
    dock_border: &SolidColorBuffer,

    text: &ShellTextState,
    tray: &TrayState,
    current_workspace: usize,
    workspace_count: usize,

    scale: Scale<f64>,
) -> Vec<ChromeRenderElement> {
    let mut elements = Vec::new();

    // ------------------------------------------------------------
    // TOP BAR + CLOCK
    // ------------------------------------------------------------
    if let Some(panel) = shell.top_bar.as_ref() {
        elements.extend(collect_top_bar_elements(
            panel, top_bar_glass,
            top_bar_shadow, top_bar_highlight, top_bar_border,
            renderer, scale,
        ));

        if let Some(clock) = text.clock_texture.as_ref() {
            let x = panel.position.0 + panel.size.0 - clock.size.w - 12;
            let y = panel.position.1 + (panel.size.1 - clock.size.h) / 2;

            if let Ok(el) = clock.element(renderer, (x, y)) {
                elements.push(ChromeRenderElement::Text(el));
            }
        }
      // Tray icons, left of the clock
        let clock_w = text
            .clock_texture
            .as_ref()
            .map(|t| t.size.w)
            .unwrap_or(0);

        let mut x = panel.position.0 + panel.size.0
            - 12 - clock_w - 16 - tray.total_width();

        let cy = panel.position.1 + panel.size.1 / 2;

        for tex in [&tray.net_tex, &tray.vol_tex, &tray.bat_tex]
            .into_iter()
            .flatten()
        {
            if let Ok(el) = tex.element(renderer, (x, cy - tex.size.h / 2)) {
                elements.push(ChromeRenderElement::Text(el));
            }

            x += tex.size.w + 10;
        }

                // Workspace Dots (Centered in top bar)
        let dot_size = 6;
        let dot_spacing = 12;
        let total_dots_w = (workspace_count as i32 * dot_size) + ((workspace_count as i32 - 1) * dot_spacing);
        let mut dot_x = panel.position.0 + (panel.size.0 / 2) - (total_dots_w / 2);
        let dot_y = panel.position.1 + (panel.size.1 / 2) - (dot_size / 2);

        for i in 0..workspace_count {
            let color = if i == current_workspace {
                crate::theme::MitosTheme::effective_accent()
            } else {
                Color32F::new(1.0, 1.0, 1.0, 0.3)
            };
            let dot = SolidColorBuffer::new((dot_size, dot_size), color);
            elements.extend(dot.render_elements(renderer, (dot_x, dot_y).into(), scale, 1.0));
            dot_x += dot_size + dot_spacing;
        }
    }

    // ------------------------------------------------------------
    // LAUNCHER + SEARCH UI
    // ------------------------------------------------------------
    if shell.launcher_visible {
        if let Some(panel) = shell.launcher.as_ref() {
            elements.extend(collect_launcher_elements(
                panel, launcher_glass,
                top_bar_shadow, top_bar_highlight, top_bar_border,
                renderer, scale,
            ));

            let (px, py) = panel.position;
            let (pw, ph) = panel.size;

            // Search query
            if let Some(q) = text.query_texture.as_ref() {
                if let Ok(el) = q.element(renderer, (px + 24, py + 22)) {
                    elements.push(ChromeRenderElement::Text(el));
                }
            }

            // Results list with selection highlight
            let row_h = 36;
            let list_top = py + 64;

            for (i, tex) in text.result_textures.iter().enumerate() {
                let row_y = list_top + i as i32 * row_h;

                if row_y + row_h > py + ph - 8 {
                    break;
                }

                if i == shell.launcher_selected {
                    let accent = crate::theme::MitosTheme::effective_accent();

                    let hl = SolidColorBuffer::new(
                        (pw - 24, row_h - 4),
                        Color32F::new(accent.r, accent.g, accent.b, 0.25),
                    );

                    elements.extend(hl.render_elements(
                        renderer,
                        (px + 12, row_y + 2).into(),
                        scale,
                        1.0,
                    ));
                }

                if let Some(t) = tex {
                    if let Ok(el) = t.element(renderer, (px + 24, row_y + 6)) {
                        elements.push(ChromeRenderElement::Text(el));
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------
    // DOCK
    // ------------------------------------------------------------
    if let Some(panel) = shell.dock.as_ref() {
        elements.extend(collect_dock_elements(
            panel, dock_layout, dock_glass,
            dock_shadow, dock_highlight, dock_border,
            renderer, scale, pointer.0,
        ));
    }

    elements
}


// ============================================================================
// WINDOW CHROME (SHADOWS & BORDERS)
// ============================================================================

/// Generate a soft drop-shadow image on the CPU.
/// We use a simple distance-field approach for speed.
fn generate_shadow_image(
    width: i32, 
    height: i32, 
    radius: f32, 
    spread: f32,
    color: (u8, u8, u8, u8)
) -> image::RgbaImage {
    let w = width.max(1) as u32;
    let h = height.max(1) as u32;
    let mut img = image::RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 0]));
    
    let (r, g, b, a) = color;
    let cx = w as f32 / 2.0;
    let cy = h as f32 / 2.0;
    let inner_w = (w as f32 - spread * 2.0).max(0.0) / 2.0;
    let inner_h = (h as f32 - spread * 2.0).max(0.0) / 2.0;

    for y in 0..h {
        for x in 0..w {
            let dx = (x as f32 - cx).abs() - inner_w;
            let dy = (y as f32 - cy).abs() - inner_h;
            
            let dist = if dx > 0.0 && dy > 0.0 {
                (dx * dx + dy * dy).sqrt()
            } else {
                dx.max(dy)
            };

            if dist < radius {
                let alpha = (1.0 - (dist / radius)) * (a as f32 / 255.0);
                let alpha_u8 = (alpha * 255.0).clamp(0.0, 255.0) as u8;
                img.put_pixel(x, y, image::Rgba([r, g, b, alpha_u8]));
            }
        }
    }
    img
}

/// Caches the window shadow texture so we don't regenerate it every frame.
pub struct WindowChrome {
    shadow_buffer: Option<MemoryRenderBuffer>,
    shadow_size: Size<i32, Logical>,
}

impl WindowChrome {
    pub fn new() -> Self {
        Self {
            shadow_buffer: None,
            shadow_size: Size::from((0, 0)),
        }
    }

    /// Ensure the shadow texture matches the requested size.
    pub fn ensure_shadow(&mut self, width: i32, height: i32) {
        // Add padding for the shadow spread
        let pad = 24; 
        let sw = width + pad * 2;
        let sh = height + pad * 2;

        if self.shadow_size.w == sw && self.shadow_size.h == sh {
            return;
        }

        let img = generate_shadow_image(sw, sh, 16.0, 8.0, (0, 0, 0, 180));
        let size = Size::<i32, Logical>::new(sw, sh);
        
        let buffer = MemoryRenderBuffer::from_slice(
            img.as_raw(),
            Fourcc::Abgr8888,
            size,
            1,
            Transform::Normal,
            Some(vec![Rectangle::from_size(size)]),
        );

        self.shadow_buffer = Some(buffer);
        self.shadow_size = size;
    }
}

/// Wrap a Wayland window with MITOS shadows and borders.
pub fn collect_window_chrome_elements(
    renderer: &mut GlesRenderer,
    window: &Window,
    location: Point<i32, Logical>,
    scale: Scale<f64>,
    chrome: &mut WindowChrome,
) -> Vec<ChromeRenderElement> {
    let mut elements = Vec::new();
    let geo = window.geometry();
    
    // 1. Shadow
    chrome.ensure_shadow(geo.size.w, geo.size.h);
    if let Some(buf) = &chrome.shadow_buffer {
        let pad = 24;
        let shadow_loc = Point::from((location.x - pad, location.y - pad));
        
        if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            shadow_loc.to_f64(),
            buf,
            Some(1.0),
            None,
            None,
            Kind::Unspecified,
        ) {
            elements.push(ChromeRenderElement::Wallpaper(el)); // Reusing Wallpaper variant for Memory buffers
        }
    }

    // 2. Window Border (1px solid accent/glass border)
    let border_color = crate::theme::MitosTheme::BORDER;
    let border_buf = SolidColorBuffer::new(
        (geo.size.w + 2, geo.size.h + 2),
        Color32F::new(border_color.r, border_color.g, border_color.b, border_color.a * 0.5),
    );
    
    let border_loc = Point::from((location.x - 1, location.y - 1));
    elements.extend(border_buf.render_elements(
        renderer,
        border_loc,
        scale,
        1.0,
    ));

    elements
}

// ============================================================================
// NOTIFICATIONS (STAGE 6)
// ============================================================================

pub fn collect_notification_elements(
    renderer: &mut GlesRenderer,
    notifications: &[crate::notifications::Notification],
    output_size: Size<i32, Logical>,
    top_bar_height: i32,
    scale: Scale<f64>,
) -> Vec<ChromeRenderElement> {
    let mut elements = Vec::new();
    
    let panel_w = 320;
    let panel_h = 80;
    let margin = 12;
    
    let start_x = output_size.w - panel_w - margin;
    let mut current_y = top_bar_height + margin;

    for notif in notifications {
        let bg_color = crate::theme::MitosTheme::effective_glass();
        let bg = SolidColorBuffer::new(
            (panel_w, panel_h),
            Color32F::new(bg_color.r, bg_color.g, bg_color.b, bg_color.a * 0.85),
        );
        
        elements.extend(bg.render_elements(
            renderer,
            (start_x, current_y).into(),
            scale,
            1.0,
        ));

        let border_color = crate::theme::MitosTheme::BORDER;
        let border = SolidColorBuffer::new(
            (panel_w, 1),
            Color32F::new(border_color.r, border_color.g, border_color.b, border_color.a),
        );
        elements.extend(border.render_elements(
            renderer,
            (start_x, current_y + panel_h - 1).into(),
            scale,
            1.0,
        ));

        if let Some(tex) = &notif.title_tex {
            if let Ok(el) = tex.element(renderer, (start_x + 16, current_y + 16)) {
                elements.push(ChromeRenderElement::Text(el));
            }
        }

        if let Some(tex) = &notif.body_tex {
            if let Ok(el) = tex.element(renderer, (start_x + 16, current_y + 40)) {
                elements.push(ChromeRenderElement::Text(el));
            }
        }

        current_y += panel_h + margin;
    }

    elements
}

pub fn collect_auth_elements(
    renderer: &mut GlesRenderer,
    auth: &crate::auth::AuthPrompt,
    output_size: Size<i32, Logical>,
    scale: Scale<f64>,
) -> Vec<ChromeRenderElement> {
    let mut elements = Vec::new();
    if !auth.active { return elements; }

    let w = 400;
    let h = 220;
    let x = (output_size.w - w) / 2;
    let y = (output_size.h - h) / 2;

    let dim = SolidColorBuffer::new(output_size, Color32F::new(0.0, 0.0, 0.0, 0.6));
    elements.extend(dim.render_elements(renderer, (0, 0).into(), scale, 1.0));

    let bg_color = crate::theme::MitosTheme::effective_glass();
    let bg = SolidColorBuffer::new((w, h), Color32F::new(bg_color.r, bg_color.g, bg_color.b, 0.95));
    elements.extend(bg.render_elements(renderer, (x, y).into(), scale, 1.0));

    let border = crate::theme::MitosTheme::BORDER;
    let b_buf = SolidColorBuffer::new((w, 1), Color32F::new(border.r, border.g, border.b, border.a));
    elements.extend(b_buf.render_elements(renderer, (x, y + h - 1).into(), scale, 1.0));

    let field_w = w - 40;
    let field_h = 40;
    let field_x = x + 20;
    let field_y = y + 120;
    
    let field_bg = SolidColorBuffer::new((field_w, field_h), Color32F::new(0.0, 0.0, 0.0, 0.3));
    elements.extend(field_bg.render_elements(renderer, (field_x, field_y).into(), scale, 1.0));

    let dot_color = Color32F::new(1.0, 1.0, 1.0, 1.0);
    let dot_size = 8;
    let dot_spacing = 16;
    
    for (i, _) in auth.password.chars().enumerate() {
        if i >= 20 { break; } 
        
        let dot_x = field_x + 15 + (i as i32 * dot_spacing);
        let dot_y = field_y + (field_h / 2) - (dot_size / 2);
        
        let dot = SolidColorBuffer::new((dot_size, dot_size), dot_color);
        elements.extend(dot.render_elements(renderer, (dot_x, dot_y).into(), scale, 1.0));
    }

    elements
}

// ============================================================================
// ON-SCREEN DISPLAY (OSD)
// ============================================================================

pub fn collect_osd_elements(
    renderer: &mut GlesRenderer,
    osd: &crate::state::OsdState,
    output_size: Size<i32, Logical>,
    scale: Scale<f64>,
) -> Vec<ChromeRenderElement> {
    let mut elements = Vec::new();
    
    // Fade out after 2 seconds
    if !osd.active || osd.last_updated.elapsed().as_secs() >= 2 {
        return elements;
    }

    let (pill_w, pill_h) = (240, 48);
    let x = (output_size.w - pill_w) / 2;
    let y = output_size.h - 140; 

    // Glass Background
    let bg_color = crate::theme::MitosTheme::effective_glass();
    let bg = SolidColorBuffer::new(
        (pill_w, pill_h),
        Color32F::new(bg_color.r, bg_color.g, bg_color.b, bg_color.a * 0.95),
    );
    elements.extend(bg.render_elements(renderer, (x, y).into(), scale, 1.0));

    // Bottom Border
    let border_color = crate::theme::MitosTheme::BORDER;
    let border = SolidColorBuffer::new((pill_w, 1), Color32F::new(border_color.r, border_color.g, border_color.b, border_color.a));
    elements.extend(border.render_elements(renderer, (x, y + pill_h - 1).into(), scale, 1.0));

    // Progress Bar Track
    let bar_w = pill_w - 80;
    let bar_h = 8;
    let bar_x = x + 60;
    let bar_y = y + (pill_h / 2) - (bar_h / 2);
    
    let track = SolidColorBuffer::new((bar_w, bar_h), Color32F::new(1.0, 1.0, 1.0, 0.1));
    elements.extend(track.render_elements(renderer, (bar_x, bar_y).into(), scale, 1.0));

    // Progress Bar Fill
    let fill_w = (bar_w as f32 * osd.value) as i32;
    if fill_w > 0 {
        let accent = crate::theme::MitosTheme::effective_accent();
        let fill = SolidColorBuffer::new((fill_w, bar_h), Color32F::new(accent.r, accent.g, accent.b, accent.a));
        elements.extend(fill.render_elements(renderer, (bar_x, bar_y).into(), scale, 1.0));
    }

    // Icon Placeholder
    let icon_size = 24;
    let icon_x = x + 20;
    let icon_y = y + (pill_h / 2) - (icon_size / 2);
    
    let icon_color = match osd.icon {
        crate::state::OsdIcon::Muted => Color32F::new(1.0, 0.3, 0.3, 1.0), 
        _ => Color32F::new(1.0, 1.0, 1.0, 0.9), 
    };
    
    let icon = SolidColorBuffer::new((icon_size, icon_size), icon_color);
    elements.extend(icon.render_elements(renderer, (icon_x, icon_y).into(), scale, 1.0));

    elements
}

// ============================================================================
// NIGHT LIGHT (BLUE LIGHT FILTER)
// ============================================================================

pub fn collect_night_light_elements(
    renderer: &mut GlesRenderer,
    night_light: bool,
    output_size: Size<i32, Logical>,
    scale: Scale<f64>,
) -> Vec<ChromeRenderElement> {
    let mut elements = Vec::new();
    if !night_light { return elements; }

    let night_tint = Color32F::new(1.0, 0.75, 0.45, 0.15); 
    let tint_buf = SolidColorBuffer::new(output_size, night_tint);
    elements.extend(tint_buf.render_elements(renderer, (0, 0).into(), scale, 1.0));
    
    elements
}

// ============================================================================
// MASTER FRAME COMPOSITION
// ============================================================================

pub fn collect_frame_elements(
    renderer: &mut GlesRenderer,
    space: &Space<Window>,
    scale: Scale<f64>,
    wallpaper: &Wallpaper,
    output_size: Size<i32, Logical>,
    window_chrome: &mut WindowChrome,
    popups: &smithay::desktop::PopupManager,
    shell_elements: impl IntoIterator<Item = ChromeRenderElement>,
    overlay_elements: impl IntoIterator<Item = ChromeRenderElement>,
    notifications: &[crate::notifications::Notification],
    top_bar_height: i32,
    auth: &crate::auth::AuthPrompt,
    current_ws: usize,  
    swipe_x: f64,
    output_width: i32,
    osd: &crate::state::OsdState,
    night_light: bool,
) -> Result<Vec<ChromeRenderElement>, GlesError> {
    let mut elements = Vec::new();

    // ------------------------------------------------------------
    // 1. WALLPAPER
    // ------------------------------------------------------------
    let wallpaper_element = wallpaper.render_element(renderer, output_size)?;
    elements.push(ChromeRenderElement::Wallpaper(wallpaper_element));

    // ------------------------------------------------------------
    // 2. MITOS SHELL (Dock + top bar)
    // ------------------------------------------------------------
    elements.extend(shell_elements);

    // ------------------------------------------------------------
    // 3. WAYLAND APPLICATION WINDOWS (Workspace Aware)
    // ------------------------------------------------------------
    for window in space.elements().rev() {
        let win_ws = crate::wm::meta(window).workspace;
        
        let diff = win_ws as i32 - current_ws as i32;
        if diff.abs() > 1 { continue; }
        
        let Some(location) = space.element_location(window) else { continue; };
        
        let offset_x = (diff as f64 * output_width as f64) + (swipe_x * output_width as f64);
        let final_loc = Point::from((location.x as f64 + offset_x, location.y as f64));

        elements.extend(collect_window_chrome_elements(
            renderer, window, final_loc.to_i32_round(), scale, window_chrome,
        ));

        let physical_location = final_loc.to_physical(scale).to_i32_round();
        elements.extend(window.render_elements(renderer, physical_location, scale, 1.0));
    }

    // ------------------------------------------------------------
    // 4. XDG POPUPS (Menus, Tooltips)
    // ------------------------------------------------------------
    elements.extend(popups.render_elements(renderer, (0, 0).into(), scale, 1.0));

    // ------------------------------------------------------------
    // 4.5 NOTIFICATIONS (STAGE 6)
    // ------------------------------------------------------------
    elements.extend(collect_notification_elements(
        renderer, notifications, output_size, top_bar_height, scale,
    ));

    // ------------------------------------------------------------
    // 4.6 NIGHT LIGHT (EYE COMFORT)
    // ------------------------------------------------------------
    elements.extend(collect_night_light_elements(renderer, night_light, output_size, scale));

    // ------------------------------------------------------------
    // 5. MITOS OVERLAYS (Launcher, etc.)
    // ------------------------------------------------------------
    elements.extend(overlay_elements);

    // ------------------------------------------------------------
    // 6. SECURE AUTHENTICATION OVERLAY
    // ------------------------------------------------------------
    elements.extend(collect_auth_elements(renderer, auth, output_size, scale));

    // ------------------------------------------------------------
    // 7. ON-SCREEN DISPLAY (OSD)
    // ------------------------------------------------------------
    elements.extend(collect_osd_elements(renderer, osd, output_size, scale));

    Ok(elements)
}

/// Render the top bar clock text (placeholder - real text rendering needs font support).
pub fn render_top_bar_clock(
    _renderer: &mut GlesRenderer,
    _panel: &GlassPanel,
    _scale: Scale<f64>,
) -> Vec<ChromeRenderElement> {
    Vec::new()
}
