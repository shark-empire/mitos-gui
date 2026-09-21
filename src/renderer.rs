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
//! The visual shell is deliberately implemented here



use smithay::backend::renderer::Bind;
use smithay::backend::renderer::Offscreen;

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
                RenderElement,
            },
            gles::{
                element::PixelShaderElement,
                GlesError,
                GlesRenderer,
                GlesTexProgram,
                GlesTexture,
            },
            Color32F,
        },
    },
    desktop::{Space, Window},
    utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Size, Transform},
};

use crate::desktop::HomeScreenConfig;
use crate::theme::MitosTheme;


/// Captures the current scene (wallpaper + windows) into an offscreen texture.
///
/// Reserved for the upcoming true frosted-glass blur pass (see
/// `frosted_glass.rs`): it isn't wired into the render loop yet, but is
/// kept compiling and ready to use once background-capture blur lands.
pub fn capture_background<E>(
    renderer: &mut GlesRenderer,
    output_size: Size<i32, Physical>,
    elements: &[E],
) -> Result<GlesTexture, Box<dyn std::error::Error>>
where
    E: RenderElement<GlesRenderer>,
{
    // 1. Create offscreen buffer
    let buffer_size = Size::<i32, smithay::utils::Buffer>::from((output_size.w, output_size.h));
    let mut bg_texture = renderer.create_buffer(Fourcc::Abgr8888, buffer_size)?;
    let mut target = renderer.bind(&mut bg_texture)?;

    // 2. Render elements to the offscreen target (force full damage via age=0)
    let mut tracker = smithay::backend::renderer::damage::OutputDamageTracker::new(
        output_size, 1.0, Transform::Normal
    );

    tracker.render_output(
        renderer,
        &mut target,
        0,
        elements,
        [0.0, 0.0, 0.0, 1.0],
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
            top_bar_color(),
        )
    }

    /// Create the MITOS launcher, centered within `(screen_width,
    /// screen_height)` at the given `(width, height)` -- sized by the
    /// caller (`desktop::ShellLayout::calculate`) from the configured
    /// `launcher_width`/`launcher_height`, since those are adjustable
    /// via home.conf and shouldn't be overridden by a fixed size here.
    pub fn launcher(
        screen_width: i32,
        screen_height: i32,
        width: i32,
        height: i32,
    ) -> Self {
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
    /// Solid by default; becomes a two-stop gradient when home.conf sets
    /// `background_gradient_bottom` (the loaded `background` key is then
    /// the top stop).
    pub fn from_home_screen(
        home_screen: &HomeScreenConfig,
    ) -> Self {
        let top = home_screen.background;

        match home_screen.background_gradient_bottom {
            Some(bottom) => Self::gradient(
                Color32F::new(top.r, top.g, top.b, top.a),
                Color32F::new(bottom.r, bottom.g, bottom.b, bottom.a),
            ),
            None => Self::solid(Color32F::new(top.r, top.g, top.b, top.a)),
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
    match DesktopBackground::from_home_screen(home_screen).mode {
        BackgroundMode::Solid(c) => c,

        // No true gradient fill in the clear-color pipeline yet -- that
        // would need a full-screen shader, like the glass panels have.
        // Average the two stops so a configured gradient still shows up
        // as *something* rather than being silently ignored.
        BackgroundMode::Gradient { top, bottom } => Color32F::new(
            (top.r() + bottom.r()) / 2.0,
            (top.g() + bottom.g()) / 2.0,
            (top.b() + bottom.b()) / 2.0,
            (top.a() + bottom.a()) / 2.0,
        ),
    }
}

pub fn clear_color(
    home_screen: &HomeScreenConfig,
) -> Color32F {
    background_color(home_screen)
}

// ============================================================================
// RENDER ELEMENT TYPES
// ============================================================================

// Update your macro:
render_elements! {
    pub ChromeRenderElement<=GlesRenderer>;
    
    // Combine both Text and Wallpaper into a single variant
    Buffer=MemoryRenderBufferRenderElement<GlesRenderer>,
    
    Glass=PixelShaderElement,
    Frosted=crate::frosted_glass::FrostedGlassElement,
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

/// Same procedural liquid-glass look as [`create_glass_panel_element`],
/// for the authentication prompt's own background — kept as a separate
/// function (rather than adding a tint parameter to that one and
/// updating its three existing call sites) purely to keep this change
/// isolated from the already-working shell panels. `critical` bakes in
/// a reddish urgency tint instead of the normal theme glass, matching
/// what the old flat critical-prompt border used to signal on its own.
pub fn create_auth_glass_element(
    renderer: &mut GlesRenderer,
    critical: bool,
) -> Result<PixelShaderElement, GlesError> {
    let glass = MitosTheme::effective_glass();
    let radius = MitosTheme::effective_panel_radius();

    let (r, g, b, a) = if critical {
        (0.30, 0.09, 0.09, 0.94)
    } else {
        (glass.r, glass.g, glass.b, 0.94)
    };

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

    float d = sd_round_box(p - half_size, half_size, RADIUS);
    float mask = 1.0 - smoothstep(-1.0, 1.0, d);

    if (mask <= 0.001) {{
        gl_FragColor = vec4(0.0);
        return;
    }}

    float edge = smoothstep(-8.0, 0.0, d);
    float inner = 1.0 - edge;

    float top_light = smoothstep(0.15, 0.9, 1.0 - v_coords.y);

    float sheen =
        (sin((v_coords.x + v_coords.y * 0.7) * 6.28318) * 0.5 + 0.5);
    sheen = smoothstep(0.6, 1.0, sheen) * 0.06;

    float grain = (hash(p) - 0.5) * GRAIN;

    vec3 color = TINT.rgb;

    color.r += edge * 0.04;
    color.g += edge * 0.06;
    color.b += edge * 0.10;

    color += top_light * SPECULAR * 0.30;
    color += sheen;
    color += edge * inner * RIM * 0.35;
    color += grain;

    float alpha = TINT.a * mask;
    alpha = max(alpha, edge * inner * RIM * 0.45 * mask);

    gl_FragColor = vec4(color, alpha);
}}
"#,
        radius = radius,
        r = r,
        g = g,
        b = b,
        a = a,
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

/// Tint used for the per-window Liquid Glass frame. Deliberately the
/// same color the shell panels use (`glass_color()`), so windows and
/// panels read as one cohesive material rather than two different
/// "glass" looks — and so a runtime glass-tint change from home.conf
/// updates both together.
pub fn window_frame_tint_color() -> Color32F {
    glass_color()
}

/// Border/rim color for the per-window Liquid Glass frame. Uses
/// `BORDER_BRIGHT` (previously defined in the theme but never wired up
/// anywhere) rather than the flatter `BORDER` the old per-window outline
/// used, so the glass edge reads as a brighter, more light-catching rim.
pub fn window_frame_border_color() -> Color32F {
    let c = MitosTheme::BORDER_BRIGHT;
    Color32F::new(c.r, c.g, c.b, c.a)
}

/// Continuous ambient "breathing" phase, `0.0..1.0`, cycling once every
/// `period_secs` seconds. Driven off wall-clock time rather than a
/// stored `Animation`, so any number of callers can read it independently
/// without needing shared per-window animation state.
fn ambient_pulse(period_secs: f64) -> f32 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);

    ((secs * std::f64::consts::TAU / period_secs).sin() as f32) * 0.5 + 0.5
}

/// Tint for the *focused* window's Liquid Glass frame: the theme accent
/// color blended in over the normal glass, with a slow ambient pulse so
/// the active window visibly "breathes" instead of sitting static. Only
/// applied on the true-blur path — see `collect_window_glass_frame_elements`.
pub fn window_frame_focus_tint_color() -> Color32F {
    let accent = MitosTheme::effective_accent();
    let base = glass_color();
    let glow = ambient_pulse(4.0);
    let mix = 0.55 + glow * 0.25;

    Color32F::new(
base.r() + (accent.r - base.r()) * mix,
base.g() + (accent.g - base.g()) * mix,
base.b() + (accent.b - base.b()) * mix,
(base.a() + glow * 0.08).min(1.0),
    )
}

/// Border/rim color for a focused window's frame — bright accent, riding
/// the same pulse as the tint above so the whole ring breathes together.
pub fn window_frame_focus_border_color() -> Color32F {
    let accent = MitosTheme::effective_accent();
    let glow = ambient_pulse(4.0);

    Color32F::new(accent.r, accent.g, accent.b, (0.55 + glow * 0.45).min(1.0))
}

/// Procedural fallback for the per-window Liquid Glass frame, used on
/// any frame where a background capture isn't available yet (mirrors
/// `create_glass_panel_element`'s role for the shell panels — same
/// visual language: rounded mask, fresnel rim, top specular, liquid
/// sheen, chromatic edge, grain). A second, smaller rounded rect is
/// subtracted from the mask so only the window's edge draws; the
/// window's own content shows through the hole in the middle untouched.
/// The hole size is derived from the element's own `size` uniform
/// (already supplied fresh every frame by `resize()`, same as the
/// filled panel shader above) rather than a second per-frame uniform.
pub fn create_window_frame_element(
    renderer: &mut GlesRenderer,
) -> Result<PixelShaderElement, GlesError> {
    let glass = MitosTheme::effective_glass();
    let outer_radius = MitosTheme::effective_window_radius();
    let ring = MitosTheme::WINDOW_FRAME_OUTSET + MitosTheme::WINDOW_FRAME_OVERLAP;
    let inner_radius = (outer_radius - ring).max(0.0);

    let shader = format!(
        r#"
precision mediump float;

varying vec2 v_coords;
uniform vec2 size;

const float RADIUS = {radius:.8};
const float INNER_RADIUS = {inner_radius:.8};
const float RING = {ring:.8};

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

    float d_outer = sd_round_box(p - half_size, half_size, RADIUS);
    float mask_outer = 1.0 - smoothstep(-1.0, 1.0, d_outer);

    vec2 inner_half_size = max(half_size - vec2(RING), vec2(0.0));
    float d_inner = sd_round_box(p - half_size, inner_half_size, INNER_RADIUS);
    float mask_inner = 1.0 - smoothstep(-1.0, 1.0, d_inner);

    float mask = clamp(mask_outer - mask_inner, 0.0, 1.0);

    if (mask <= 0.001) {{
        gl_FragColor = vec4(0.0);
        return;
    }}

    float edge = smoothstep(-8.0, 0.0, d_outer);
    float inner = 1.0 - edge;

    float top_light = smoothstep(0.15, 0.9, 1.0 - v_coords.y);

    float sheen =
        (sin((v_coords.x + v_coords.y * 0.7) * 6.28318) * 0.5 + 0.5);
    sheen = smoothstep(0.6, 1.0, sheen) * 0.06;

    float grain = (hash(p) - 0.5) * GRAIN;

    vec3 color = TINT.rgb;

    color.r += edge * 0.04;
    color.g += edge * 0.06;
    color.b += edge * 0.10;

    color += top_light * SPECULAR * 0.30;
    color += sheen;
    color += edge * inner * RIM * 0.35;
    color += grain;

    float alpha = TINT.a * mask;
    alpha = max(alpha, edge * inner * RIM * 0.45 * mask);

    gl_FragColor = vec4(color, alpha);
}}
"#,
        radius = outer_radius,
        inner_radius = inner_radius,
        ring = ring,
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


/// How large (logical px, always kept square) the launcher's one-shot
/// "activation ring" grows to before fading out. Must match the
/// shader's own `MAX_EXTENT` constant below.
const LAUNCHER_RING_MAX_EXTENT: f32 = 260.0;

/// A one-shot expanding accent-colored ring, played once when the
/// launcher opens (see `MitosShell::launcher_anim` and the launcher
/// block in `collect_shell_elements`). Reuses the same trick as
/// `create_window_frame_element` above: the caller grows this element's
/// own bounding box every frame via the ordinary `.resize()` call
/// (already proven, low-risk), and the shader reads its own current
/// `size` against `MAX_EXTENT` to know how far along it is — no custom
/// per-frame uniform needed, so nothing here depends on exactly how
/// `PixelShaderElement::resize`'s uniform-update argument behaves.
pub fn create_launcher_ring_element(
    renderer: &mut GlesRenderer,
) -> Result<PixelShaderElement, GlesError> {
    let accent = MitosTheme::effective_accent();

    let shader = format!(
        r#"
precision mediump float;

varying vec2 v_coords;
uniform vec2 size;

const float MAX_EXTENT = {max_extent:.8};

const vec3 RING_COLOR = vec3(
    {r:.8},
    {g:.8},
    {b:.8}
);

void main() {{
    float p = clamp(size.x / MAX_EXTENT, 0.0, 1.0);

    vec2 uv = v_coords * size;
    vec2 center = size * 0.5;

    float radius = size.x * 0.5 * 0.9;
    float thickness = 2.5 + 5.0 * (1.0 - p);

    float d = abs(length(uv - center) - radius) - thickness * 0.5;
    float ring_mask = 1.0 - smoothstep(0.0, 1.5, d);

    float fade = 1.0 - smoothstep(0.55, 1.0, p);

    gl_FragColor = vec4(RING_COLOR, ring_mask * fade * 0.9);
}}
"#,
        max_extent = LAUNCHER_RING_MAX_EXTENT,
        r = accent.r,
        g = accent.g,
        b = accent.b,
    );

    let program = renderer.compile_custom_pixel_shader(shader, &[])?;

    Ok(PixelShaderElement::new(
        program,
        Rectangle::new((0, 0).into(), (1, 1).into()),
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
    bg: Option<(&GlesTexture, &GlesTexProgram)>,
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
    // Glass body: true frosted (real background blur + tint) when a
    // background capture is available this frame, falling back to the
    // procedural liquid-glass shader otherwise (e.g. the very first
    // frame, before anything has been captured yet).
    // ------------------------------------------------------------

    if let Some((bg_texture, frost_program)) = bg {
        let phys_loc = Point::<i32, Logical>::from((x, y))
            .to_f64()
            .to_physical(scale)
            .to_i32_round();
        let phys_size = Size::<i32, Logical>::from((width, height))
            .to_f64()
            .to_physical(scale)
            .to_i32_round();

        elements.push(ChromeRenderElement::Frosted(
            crate::frosted_glass::FrostedGlassElement::new(
                Rectangle::new(phys_loc, phys_size),
                bg_texture.clone(),
                frost_program.clone(),
                panel.tint.components(),
                panel.border.components(),
            ),
        ));
    } else {
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
    }

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
    bg: Option<(&GlesTexture, &GlesTexProgram)>,
    shadow_buffer: &SolidColorBuffer,
    highlight_buffer: &SolidColorBuffer,
    border_buffer: &SolidColorBuffer,
    renderer: &mut GlesRenderer,
    scale: Scale<f64>,
) -> Vec<ChromeRenderElement> {
    collect_glass_panel_elements(
        panel,
        glass_panel,
        bg,
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
    bg: Option<(&GlesTexture, &GlesTexProgram)>,
    shadow_buffer: &SolidColorBuffer,
    highlight_buffer: &SolidColorBuffer,
    border_buffer: &SolidColorBuffer,
    renderer: &mut GlesRenderer,
    scale: Scale<f64>,
) -> Vec<ChromeRenderElement> {
    collect_glass_panel_elements(
        panel,
        glass_panel,
        bg,
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
    bg: Option<(&GlesTexture, &GlesTexProgram)>,
    shadow_buffer: &SolidColorBuffer,
    highlight_buffer: &SolidColorBuffer,
    border_buffer: &SolidColorBuffer,
    renderer: &mut GlesRenderer,
    scale: Scale<f64>,
    pointer_x: f64,
) -> Vec<ChromeRenderElement> {
    let mut elements = collect_glass_panel_elements(
        panel, glass_panel, bg, shadow_buffer, highlight_buffer,
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
        // Top bar clock (changes once per minute -- the date portion
        // only actually varies once a day, but folding both into one
        // string keeps this to the single cached texture it already
        // was, re-rasterized only on the minute boundary either way)
        // --------------------------------------------------------
        let now = format!(
            "{}  {}",
            crate::shell_interaction::current_date_string(),
            crate::shell_interaction::current_time_string(),
        );

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
            self.bat_tex = battery.and_then(|b| {
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

/// Deterministic placeholder color for a launcher result row, standing in
/// for real icon-theme lookup -- MITOS doesn't parse/rasterize `.desktop`
/// icon themes yet, so each app gets a stable, distinct color swatch
/// instead of every row looking identical.
fn icon_swatch_color(key: &str) -> (f32, f32, f32) {
    let mut hash: u32 = 2166136261;
    for b in key.bytes() {
        hash ^= b as u32;
        hash = hash.wrapping_mul(16777619);
    }

    hsv_to_rgb((hash % 360) as f32, 0.45, 0.85)
}

/// Minimal HSV -> RGB conversion (`s`, `v` in 0.0..=1.0, `h` in degrees).
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;

    let (r, g, b) = match h as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    (r + m, g + m, b + m)
}

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
    // One-shot "activation ring" played when the launcher opens — see
    // `create_launcher_ring_element`.
    launcher_ring: &mut PixelShaderElement,

    // True frosted-glass background capture + each panel's compiled
    // frosted-glass shader program. `bg_texture` is `None` on frames
    // where a capture wasn't available (e.g. the very first frame),
    // in which case every panel below falls back to the procedural
    // liquid-glass shader instead.
    bg_texture: Option<&GlesTexture>,
    top_bar_frost: &GlesTexProgram,
    launcher_frost: &GlesTexProgram,
    dock_frost: &GlesTexProgram,

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
            bg_texture.map(|t| (t, top_bar_frost)),
            top_bar_shadow, top_bar_highlight, top_bar_border,
            renderer, scale,
        ));

        // Right-edge margin: inset by the panel's own corner radius so
        // the clock/tray content clears the rounded corner curve instead
        // of a fixed guess at how much that curve eats into the corner.
        let edge_margin = panel.radius.round() as i32;

        if let Some(clock) = text.clock_texture.as_ref() {
            let x = panel.position.0 + panel.size.0 - clock.size.w - edge_margin;
            let y = panel.position.1 + (panel.size.1 - clock.size.h) / 2;

            if let Ok(el) = clock.element(renderer, (x, y)) {
                elements.push(ChromeRenderElement::Buffer(el));
            }
        }
      // Tray icons, left of the clock
        let clock_w = text
            .clock_texture
            .as_ref()
            .map(|t| t.size.w)
            .unwrap_or(0);

        let mut x = panel.position.0 + panel.size.0
            - edge_margin - clock_w - 16 - tray.total_width();

        let cy = panel.position.1 + panel.size.1 / 2;

        for tex in [&tray.net_tex, &tray.vol_tex, &tray.bat_tex]
            .into_iter()
            .flatten()
        {
            if let Ok(el) = tex.element(renderer, (x, cy - tex.size.h / 2)) {
                elements.push(ChromeRenderElement::Buffer(el));
            }

            x += tex.size.w + 10;
        }

                // Workspace Dots (Centered in top bar)
        let dot_size = 6;
        let dot_spacing = crate::theme::MitosTheme::SPACING as i32;
        let total_dots_w = (workspace_count as i32 * dot_size) + ((workspace_count as i32 - 1) * dot_spacing);
        let mut dot_x = panel.position.0 + (panel.size.0 / 2) - (total_dots_w / 2);
        let dot_y = panel.position.1 + (panel.size.1 / 2) - (dot_size / 2);

        for i in 0..workspace_count {
            let color = if i == current_workspace {
                let c = crate::theme::MitosTheme::effective_accent();
                Color32F::new(c.r, c.g, c.b, c.a)
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
                bg_texture.map(|t| (t, launcher_frost)),
                top_bar_shadow, top_bar_highlight, top_bar_border,
                renderer, scale,
            ));

            // One-shot "activation ring" — plays once when the launcher
            // opens (see `MitosShell::toggle_launcher`). Purely additive
            // on top of the panel above, so it can't disturb the panel's
            // own geometry or the search/results layout below, both of
            // which key off `panel.position`/`size` directly and are
            // left completely untouched.
            if let Some(anim) = shell.launcher_anim {
                let progress = anim.progress(std::time::Instant::now());
                if !progress.is_finished() {
                    let eased = progress.ease_out_back().0.max(0.0);
                    let extent = ((LAUNCHER_RING_MAX_EXTENT * eased) as i32).max(1);

                    let cx = panel.position.0 + panel.size.0 / 2;
                    let cy = panel.position.1 + panel.size.1 / 2;

                    launcher_ring.resize(
                        Rectangle::new(
                            (cx - extent / 2, cy - extent / 2).into(),
                            (extent, extent).into(),
                        ),
                        None,
                    );

                    elements.push(ChromeRenderElement::Glass(launcher_ring.clone()));
                }
            }

            let (px, py) = panel.position;
            let (pw, ph) = panel.size;

            // Search query
            if let Some(q) = text.query_texture.as_ref() {
                if let Ok(el) = q.element(renderer, (px + 24, py + 22)) {
                    elements.push(ChromeRenderElement::Buffer(el));
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

                // Icon swatch: stands in for real icon-theme resolution
                // (see `icon_swatch_color`) so results are distinguishable
                // at a glance instead of being text-only rows.
                if let Some(app) = shell.launcher_results.get(i) {
                    let key = if app.icon.is_empty() { &app.name } else { &app.icon };
                    let (r, g, b) = icon_swatch_color(key);

                    let icon_buf = SolidColorBuffer::new(
                        (20, 20),
                        Color32F::new(r, g, b, 0.85),
                    );
                    elements.extend(icon_buf.render_elements(
                        renderer,
                        (px + 16, row_y + 8).into(),
                        scale,
                        1.0,
                    ));
                }

                if let Some(t) = tex {
                    if let Ok(el) = t.element(renderer, (px + 44, row_y + 6)) {
                        elements.push(ChromeRenderElement::Buffer(el));
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
            bg_texture.map(|t| (t, dock_frost)),
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

/// Caches the window shadow texture, plus the procedural fallback used
/// by the per-window Liquid Glass frame before any background capture
/// exists, so neither is regenerated every frame.
pub struct WindowChrome {
    shadow_buffer: Option<MemoryRenderBuffer>,
    shadow_size: Size<i32, Logical>,
    frame_fallback: Option<PixelShaderElement>,
}

impl WindowChrome {
    pub fn new() -> Self {
        Self {
            shadow_buffer: None,
            shadow_size: Size::from((0, 0)),
            frame_fallback: None,
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

        let img = generate_shadow_image(
            sw, sh, crate::theme::MitosTheme::SHADOW_RADIUS, 8.0, (0, 0, 0, 180),
        );
        let size = Size::<i32, Logical>::new(sw, sh);
        let buffer_size = Size::<i32, Buffer>::from((sw, sh));

        let buffer = MemoryRenderBuffer::from_slice(
            img.as_raw(),
            Fourcc::Abgr8888,
            buffer_size,
            1,
            Transform::Normal,
            Some(vec![Rectangle::from_size(buffer_size)]),
        );

        self.shadow_buffer = Some(buffer);
        self.shadow_size = size;
    }

    /// (Re)compile the procedural fallback used for the per-window glass
    /// frame before any background capture is available this run. Called
    /// once at startup and again whenever the theme reloads (the window
    /// radius may have changed), mirroring how the shell's procedural
    /// glass panels get recompiled. A failure here just leaves windows
    /// without a frame until the next successful call — never fatal.
    pub fn refresh_frame_fallback(&mut self, renderer: &mut GlesRenderer) {
        if let Ok(el) = create_window_frame_element(renderer) {
            self.frame_fallback = Some(el);
        }
    }
}

/// Wrap a Wayland window with MITOS's drop shadow. Skipped for fullscreen
/// windows — the window already fills the whole output, so a shadow
/// would only be wasted off-screen work.
///
/// (The old flat 1px border that used to live here has been replaced by
/// the true Liquid Glass frame in `collect_window_glass_frame_elements`,
/// drawn after the window's own content — see that function.)
pub fn collect_window_chrome_elements(
    renderer: &mut GlesRenderer,
    window: &Window,
    location: Point<i32, Logical>,
    _scale: Scale<f64>,
    chrome: &mut WindowChrome,
) -> Vec<ChromeRenderElement> {
    let mut elements = Vec::new();

    if crate::wm::meta(window).fullscreen {
        return elements;
    }

    let geo = window.geometry();

    // Shadow
    chrome.ensure_shadow(geo.size.w, geo.size.h);
    if let Some(buf) = &chrome.shadow_buffer {
        let pad = 24;
        // A slight directional drop -- light reads as coming from above,
        // rather than a shadow spread evenly on all sides.
        let shadow_loc = Point::from((
            location.x - pad + crate::theme::MitosTheme::SHADOW_OFFSET_X as i32,
            location.y - pad + crate::theme::MitosTheme::SHADOW_OFFSET_Y as i32,
        ));
        
        if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            shadow_loc.to_f64(),
            buf,
            Some(1.0),
            None,
            None,
            Kind::Unspecified,
        ) {
            elements.push(ChromeRenderElement::Buffer(el));
        }
    }

    elements
}

/// Per-window Liquid Glass frame: a thin frosted-glass ring around a
/// window's edge, sampling the same offscreen background capture the
/// shell panels use so it shows a genuine blur of whatever actually
/// sits behind that window (wallpaper, or another window further back).
/// Falls back to the procedural ring shader (`create_window_frame_element`,
/// cached on `chrome`) on any frame where a capture isn't available.
///
/// Callers must draw this *after* the window's own content: the ring's
/// rounded outer edge is meant to visually cover the client's square
/// corners, which only works if it's painted on top. This is pure
/// compositor-side chrome — it never reads or needs to know anything
/// about the window's own pixels — so it applies the same way to a
/// MITOS-native window and an unmodified third-party Wayland client.
///
/// Skipped for fullscreen windows, same as the shadow above.
pub fn collect_window_glass_frame_elements(
    window: &Window,
    location: Point<i32, Logical>,
    scale: Scale<f64>,
    bg: Option<(&GlesTexture, &GlesTexProgram)>,
    chrome: &mut WindowChrome,
    focused: bool,
) -> Vec<ChromeRenderElement> {
    let mut elements = Vec::new();

    if crate::wm::meta(window).fullscreen {
        return elements;
    }

    let geo = window.geometry();
    let outset = MitosTheme::WINDOW_FRAME_OUTSET.round() as i32;

    let frame_loc = Point::from((location.x - outset, location.y - outset));
    let frame_size = Size::from((geo.size.w + outset * 2, geo.size.h + outset * 2));

    if frame_size.w <= 0 || frame_size.h <= 0 {
        return elements;
    }

    if let Some((bg_texture, program)) = bg {
        let phys_loc = frame_loc
            .to_f64()
            .to_physical(scale)
            .to_i32_round();
        let phys_size = frame_size
            .to_f64()
            .to_physical(scale)
            .to_i32_round();

        // The focused window gets the accent-colored, slowly-pulsing
        // glow; every other window gets the normal neutral glass edge.
        // (The rare procedural-fallback path below doesn't distinguish
        // the two -- see its doc comment.)
        let (mut tint, mut border) = if focused {
            (window_frame_focus_tint_color(), window_frame_focus_border_color())
        } else {
            (window_frame_tint_color(), window_frame_border_color())
        };

        // Brief "materialize" flash the moment a window is first mapped,
        // fading out over MATERIALIZE_SECS -- purely by brightening this
        // same ring for a moment, so a freshly-opened window reads as
        // *arriving* rather than just popping into existence. Reuses
        // `WindowMeta::mapped_at` (wm.rs) and applies here regardless of
        // `focused`/client identity, same as the ring itself.
        const MATERIALIZE_SECS: f32 = 0.45;
        let since_mapped = crate::wm::meta(window).mapped_at.elapsed().as_secs_f32();
        if since_mapped < MATERIALIZE_SECS {
            let decay = 1.0 - (since_mapped / MATERIALIZE_SECS);
            let boost = decay * decay; // eases the fade-out rather than a linear ramp-down
            let accent = MitosTheme::effective_accent();

            let lerp = crate::animation::lerp;
            let p = crate::animation::Progress;

            tint = Color32F::new(
                lerp(tint.r(), accent.r, p(boost * 0.6)),
                lerp(tint.g(), accent.g, p(boost * 0.6)),
                lerp(tint.b(), accent.b, p(boost * 0.6)),
                lerp(tint.a(), 1.0, p(boost * 0.15)),
            );
            border = Color32F::new(
                lerp(border.r(), 1.0, p(boost)),
                lerp(border.g(), 1.0, p(boost)),
                lerp(border.b(), 1.0, p(boost)),
                lerp(border.a(), 1.0, p(boost * 0.5)),
            );
        }

        elements.push(ChromeRenderElement::Frosted(
            crate::frosted_glass::FrostedGlassElement::new(
                Rectangle::new(phys_loc, phys_size),
                bg_texture.clone(),
                program.clone(),
                tint.components(),
                border.components(),
            ),
        ));
    } else if let Some(fallback) = chrome.frame_fallback.as_mut() {
        fallback.resize(
            Rectangle::new(frame_loc, frame_size),
            None,
        );

        elements.push(ChromeRenderElement::Glass(fallback.clone()));
    }

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
    notification_glass: &mut PixelShaderElement,
) -> Vec<ChromeRenderElement> {
    let mut elements = Vec::new();
    
    let panel_w = 320;
    let panel_h = 80;
    let margin = 12;
    
    let start_x = output_size.w - panel_w - margin;
    let mut current_y = top_bar_height + margin;

    // A thin breathing accent edge on the leading side, replacing the
    // old flat bottom rule -- the same "this is alive" language the
    // windows and lock screen now use, rather than notifications being
    // the one surface left looking static.
    let pulse = ambient_pulse(3.5);
    let accent = crate::theme::MitosTheme::effective_accent();
    let edge_color = Color32F::new(accent.r, accent.g, accent.b, 0.25 + pulse * 0.25);

    for notif in notifications {
        // Soft shadow beneath the toast so it reads as floating above
        // the desktop, matching the depth language the shell panels
        // already have (`shadow_color()` / `collect_glass_panel_elements`)
        // -- toasts just never got their own shadow, since they're built
        // fresh each frame instead of sharing a persistent buffer.
        let soft = crate::theme::MitosTheme::SHADOW_SOFT;
        let shadow = SolidColorBuffer::new(
            (panel_w, panel_h),
            Color32F::new(soft.r, soft.g, soft.b, soft.a),
        );
        elements.extend(shadow.render_elements(
            renderer,
            (start_x + 3, current_y + 4).into(),
            scale,
            1.0,
        ));

        notification_glass.resize(
            Rectangle::new((start_x, current_y).into(), (panel_w, panel_h).into()),
            None,
        );
        elements.push(ChromeRenderElement::Glass(notification_glass.clone()));

        let edge = SolidColorBuffer::new((3, panel_h), edge_color);
        elements.extend(edge.render_elements(renderer, (start_x, current_y).into(), scale, 1.0));

        if let Some(tex) = &notif.title_tex {
            if let Ok(el) = tex.element(renderer, (start_x + 16, current_y + 16)) {
                elements.push(ChromeRenderElement::Buffer(el));
            }
        }

        if let Some(tex) = &notif.body_tex {
            if let Ok(el) = tex.element(renderer, (start_x + 16, current_y + 40)) {
                elements.push(ChromeRenderElement::Buffer(el));
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
    auth_glass: &mut PixelShaderElement,
    auth_glass_critical: &mut PixelShaderElement,
) -> Vec<ChromeRenderElement> {
    let mut elements = Vec::new();
    if !auth.active { return elements; }

    let w = 400;
    let h = 220;
    let x = (output_size.w - w) / 2;
    let y = (output_size.h - h) / 2;

    let dim = SolidColorBuffer::new(output_size, Color32F::new(0.0, 0.0, 0.0, 0.6));
    elements.extend(dim.render_elements(renderer, (0, 0).into(), scale, 1.0));

    // Liquid Glass background — same procedural material as the shell
    // panels, swapped to a reddish tint for a critical-risk prompt (see
    // `create_auth_glass_element`). Replaces the old flat SolidColorBuffer.
    let glass_el = if auth.critical { auth_glass_critical } else { auth_glass };
    glass_el.resize(Rectangle::new((x, y).into(), (w, h).into()), None);
    elements.push(ChromeRenderElement::Glass(glass_el.clone()));

    // A thin glowing outline all the way around, breathing slowly — red
    // for a critical-risk elevation prompt, accent for everything else
    // — so it visibly reads as more urgent before a word of the text is
    // read. Replaces the old bottom-only (plus conditional top) flat rule.
    let pulse = ambient_pulse(3.0);
    let border = if auth.critical {
        Color32F::new(0.85, 0.25, 0.25, 0.6 + pulse * 0.4)
    } else {
        let a = crate::theme::MitosTheme::effective_accent();
        Color32F::new(a.r, a.g, a.b, 0.35 + pulse * 0.35)
    };
    let edge = 2;
    let top_edge = SolidColorBuffer::new((w, edge), border);
    elements.extend(top_edge.render_elements(renderer, (x, y).into(), scale, 1.0));
    let bottom_edge = SolidColorBuffer::new((w, edge), border);
    elements.extend(bottom_edge.render_elements(renderer, (x, y + h - edge).into(), scale, 1.0));
    let left_edge = SolidColorBuffer::new((edge, h), border);
    elements.extend(left_edge.render_elements(renderer, (x, y).into(), scale, 1.0));
    let right_edge = SolidColorBuffer::new((edge, h), border);
    elements.extend(right_edge.render_elements(renderer, (x + w - edge, y).into(), scale, 1.0));

    // Title ("Locked" / the requesting app's name) and subtitle (the
    // lock reason / "<action> · <risk> · <duration>") -- cached on
    // `auth` itself and only re-rasterized when they actually change,
    // see `AuthPrompt::refresh_title_textures`.
    if let Some(tex) = &auth.title_tex {
        if let Ok(el) = tex.element(renderer, (x + 20, y + 22)) {
            elements.push(ChromeRenderElement::Buffer(el));
        }
    }
    if let Some(tex) = &auth.subtitle_tex {
        if let Ok(el) = tex.element(renderer, (x + 20, y + 50)) {
            elements.push(ChromeRenderElement::Buffer(el));
        }
    }

    let field_w = w - 40;
    let field_h = 40;
    let field_x = x + 20;
    let field_y = y + 120;
    
    let field_bg_color = if auth.error_msg.is_some() {
        Color32F::new(0.6, 0.15, 0.15, 0.35) // reddish flash on a rejected attempt
    } else if auth.pending {
        Color32F::new(0.0, 0.0, 0.0, 0.15) // dimmed while mitos-session checks
    } else {
        Color32F::new(0.0, 0.0, 0.0, 0.3)
    };
    let field_bg = SolidColorBuffer::new((field_w, field_h), field_bg_color);
    elements.extend(field_bg.render_elements(renderer, (field_x, field_y).into(), scale, 1.0));

    // While mitos-session is checking the attempt, a thin pulsing accent
    // line under the field reads as "the system is thinking" rather than
    // just a dimmed, static box.
    if auth.pending {
        let a = crate::theme::MitosTheme::effective_accent();
        let glow = ambient_pulse(1.2);
        let line = SolidColorBuffer::new(
            (field_w, 2),
            Color32F::new(a.r, a.g, a.b, 0.3 + glow * 0.5),
        );
        elements.extend(line.render_elements(renderer, (field_x, field_y + field_h - 2).into(), scale, 1.0));
    }

    let accent = crate::theme::MitosTheme::effective_accent();
    let dot_color = Color32F::new(
        0.75 + accent.r * 0.25,
        0.75 + accent.g * 0.25,
        0.75 + accent.b * 0.25,
        1.0,
    );
    let dot_size = 8;
    let dot_spacing = 16;
    
    for (i, _) in auth.password.chars().enumerate() {
        if i >= 20 { break; } 
        
        let dot_x = field_x + 15 + (i as i32 * dot_spacing);
        let dot_y = field_y + (field_h / 2) - (dot_size / 2);
        
        let dot = SolidColorBuffer::new((dot_size, dot_size), dot_color);
        elements.extend(dot.render_elements(renderer, (dot_x, dot_y).into(), scale, 1.0));
    }

    // Below the field: the error from the last attempt if there is
    // one, otherwise (for an elevation prompt only -- the lock screen
    // has no cancel) a reminder that Escape declines. Rendered fresh
    // each frame rather than cached: unlike the title/subtitle above,
    // `error_msg` can be set by a direct field write from
    // `poll_session_ipc` (mirroring how the rest of this struct
    // already works), so there's no single choke point to hook a
    // cache-refresh into -- and a short line of text shown for at
    // most a few seconds is cheap enough to just re-rasterize.
    let hint_y = field_y + field_h + 14;
    if let Some(err) = &auth.error_msg {
        if let Some(img) = auth.text_renderer.render(err, 13.0, (255, 150, 150, 255)) {
            if let Some(tex) = crate::text::TextTexture::from_rgba(img) {
                if let Ok(el) = tex.element(renderer, (field_x, hint_y)) {
                    elements.push(ChromeRenderElement::Buffer(el));
                }
            }
        }
    } else if auth.request_id.is_some() {
        if let Some(img) = auth.text_renderer.render("Enter to allow  ·  Esc to decline", 13.0, (170, 170, 170, 220)) {
            if let Some(tex) = crate::text::TextTexture::from_rgba(img) {
                if let Ok(el) = tex.element(renderer, (field_x, hint_y)) {
                    elements.push(ChromeRenderElement::Buffer(el));
                }
            }
        }
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
        Color32F::new(
            bg_color.r,
            bg_color.g,
            bg_color.b,
            crate::theme::MitosTheme::effective_glass_alpha() * 0.95,
        ),
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
    anim: &crate::animation::Animation,
    output_size: Size<i32, Logical>,
    scale: Scale<f64>,
) -> Vec<ChromeRenderElement> {
    let mut elements = Vec::new();

    let now = std::time::Instant::now();

    // Once the fade has settled and the filter is off, there's nothing to
    // draw -- skip the eased-alpha computation below entirely rather than
    // spending it on a tint nobody will see.
    if !night_light && anim.finished(now) {
        return elements;
    }

    let progress = anim.progress(now).ease_in_out().0;
    let alpha = if night_light { progress } else { 1.0 - progress };

    let night_tint = Color32F::new(1.0, 0.75, 0.45, 0.15 * alpha);
    let tint_buf = SolidColorBuffer::new(output_size, night_tint);
    elements.extend(tint_buf.render_elements(renderer, (0, 0).into(), scale, 1.0));
    
    elements
}

// ============================================================================
// MASTER FRAME COMPOSITION
// ============================================================================

/// Collect just the desktop background — wallpaper + application windows,
/// with no shell chrome, notifications, or overlays.
///
/// This is the same content [`collect_frame_elements`] draws as its own
/// first two steps, but exposed standalone so the caller can render it to
/// an offscreen texture via [`capture_background`] *before* building the
/// shell panels, giving [`collect_shell_elements`] something real to blur
/// for the true frosted-glass effect (see `frosted_glass.rs`). Called a
/// second time inside `collect_frame_elements` itself for the actual
/// on-screen frame — cheap to recompute (no GPU work, just element
/// descriptors) and keeps that function's signature unchanged.
pub fn collect_background_elements(
    renderer: &mut GlesRenderer,
    space: &Space<Window>,
    scale: Scale<f64>,
    wallpaper: &Wallpaper,
    output_size: Size<i32, Logical>,
    window_chrome: &mut WindowChrome,
    current_ws: usize,
    output_name: &str,
    swipe_x: f64,
    output_width: i32,
) -> Result<Vec<ChromeRenderElement>, GlesError> {
    let mut elements = Vec::new();

    let wallpaper_element = wallpaper.render_element(renderer, output_size)?;
    elements.push(ChromeRenderElement::Buffer(wallpaper_element));

    for window in space.elements().rev() {
        let win_ws = crate::wm::meta(window).workspace.get(output_name).copied().unwrap_or(0);

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

    Ok(elements)
}

pub fn collect_frame_elements(
    renderer: &mut GlesRenderer,
    space: &Space<Window>,
    scale: Scale<f64>,
    wallpaper: &Wallpaper,
    output_size: Size<i32, Logical>,
    window_chrome: &mut WindowChrome,
    // True frosted-glass background capture + the per-window glass
    // frame's compiled shader program — same `None`-on-first-frame
    // fallback contract as the shell panels' `bg_texture`/`*_frost`
    // pairs passed into `collect_shell_elements`.
    window_frame_bg_texture: Option<&GlesTexture>,
    window_frame_frost: &GlesTexProgram,
    // Which window (if any) currently holds keyboard focus -- drives the
    // accent-glow variant of the glass frame in the loop below.
    focused_window: Option<&Window>,
    _popups: &smithay::desktop::PopupManager,
    shell_elements: impl IntoIterator<Item = ChromeRenderElement>,
    overlay_elements: impl IntoIterator<Item = ChromeRenderElement>,
    notifications: &[crate::notifications::Notification],
    notification_glass: &mut PixelShaderElement,
    top_bar_height: i32,
    auth: &crate::auth::AuthPrompt,
    auth_glass: &mut PixelShaderElement,
    auth_glass_critical: &mut PixelShaderElement,
    current_ws: usize,
    output_name: &str,
    swipe_x: f64,
    output_width: i32,
    osd: &crate::state::OsdState,
    night_light: bool,
    night_light_anim: &crate::animation::Animation,
) -> Result<Vec<ChromeRenderElement>, GlesError> {
    let mut elements = Vec::new();

    // ------------------------------------------------------------
    // 1. WALLPAPER
    // ------------------------------------------------------------
    let wallpaper_element = wallpaper.render_element(renderer, output_size)?;

    elements.push(ChromeRenderElement::Buffer(wallpaper_element));

    // ------------------------------------------------------------
    // 2. MITOS SHELL (Dock + top bar)
    // ------------------------------------------------------------
    elements.extend(shell_elements);

    // ------------------------------------------------------------
    // 3. WAYLAND APPLICATION WINDOWS (Workspace Aware)
    // ------------------------------------------------------------
    for window in space.elements().rev() {
        let win_ws = crate::wm::meta(window).workspace.get(output_name).copied().unwrap_or(0);

        let diff = win_ws as i32 - current_ws as i32;
        if diff.abs() > 1 { continue; }
        
        let Some(location) = space.element_location(window) else { continue; };
        
        let offset_x = (diff as f64 * output_width as f64) + (swipe_x * output_width as f64);
        let final_loc = Point::from((location.x as f64 + offset_x, location.y as f64));
        let final_loc_i32 = final_loc.to_i32_round();

        elements.extend(collect_window_chrome_elements(
            renderer, window, final_loc_i32, scale, window_chrome,
        ));

        let physical_location = final_loc.to_physical(scale).to_i32_round();
        elements.extend(window.render_elements(renderer, physical_location, scale, 1.0));

        // Liquid Glass window frame — drawn last so its rounded, frosted
        // edge sits on top of the window's own (square-cornered) content
        // and visually rounds it off. See `collect_window_glass_frame_elements`.
        elements.extend(collect_window_glass_frame_elements(
            window,
            final_loc_i32,
            scale,
            window_frame_bg_texture.map(|t| (t, window_frame_frost)),
            window_chrome,
            focused_window == Some(window),
        ));
    }

    // ------------------------------------------------------------
    // 4. XDG POPUPS (Menus, Tooltips)
    // ------------------------------------------------------------
    // Note: smithay 0.7's `PopupManager` has no API to iterate every
    // tracked popup directly (only `popups_for_surface(surface)` for a
    // specific surface). Popups belonging to each mapped window are
    // already included above via `Window::render_elements`, which walks
    // that window's popup tree internally, so no separate pass is needed
    // here.


    // ------------------------------------------------------------
    // 4.5 NOTIFICATIONS (STAGE 6)
    // ------------------------------------------------------------
    elements.extend(collect_notification_elements(
        renderer, notifications, output_size, top_bar_height, scale, notification_glass,
    ));

    // ------------------------------------------------------------
    // 4.6 NIGHT LIGHT (EYE COMFORT)
    // ------------------------------------------------------------
    elements.extend(collect_night_light_elements(renderer, night_light, night_light_anim, output_size, scale));

    // ------------------------------------------------------------
    // 5. MITOS OVERLAYS (Launcher, etc.)
    // ------------------------------------------------------------
    elements.extend(overlay_elements);

    // ------------------------------------------------------------
    // 6. SECURE AUTHENTICATION OVERLAY
    // ------------------------------------------------------------
    elements.extend(collect_auth_elements(renderer, auth, output_size, scale, auth_glass, auth_glass_critical));

    // ------------------------------------------------------------
    // 7. ON-SCREEN DISPLAY (OSD)
    // ------------------------------------------------------------
    elements.extend(collect_osd_elements(renderer, osd, output_size, scale));

    Ok(elements)
}
