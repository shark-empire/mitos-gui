//! MITOS visual design system.
//!
//! Centralized theme values keep the desktop consistent and make it
//! possible to change the appearance without touching compositor logic.

#[derive(Clone, Copy, Debug)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }
}

pub struct MitosTheme;

impl MitosTheme {
    // ------------------------------------------------------------
    // Desktop
    // ------------------------------------------------------------

    pub const BACKGROUND: Color =
        Color::rgba(0.025, 0.035, 0.055, 1.0);

    // ------------------------------------------------------------
    // Glass
    // ------------------------------------------------------------

    /// Main MITOS glass surface.
    pub const GLASS: Color =
        Color::rgba(0.10, 0.13, 0.18, 0.72);

    /// Lighter glass highlight.
    pub const GLASS_LIGHT: Color =
        Color::rgba(0.18, 0.22, 0.30, 0.45);

    /// Very subtle glass highlight used near the top edge.
    pub const GLASS_HIGHLIGHT: Color =
        Color::rgba(0.80, 0.88, 1.0, 0.10);

    // ------------------------------------------------------------
    // Borders
    // ------------------------------------------------------------

    pub const BORDER: Color =
        Color::rgba(0.55, 0.65, 0.80, 0.20);

    pub const BORDER_BRIGHT: Color =
        Color::rgba(0.75, 0.85, 1.0, 0.32);

    // ------------------------------------------------------------
    // Shadows
    // ------------------------------------------------------------

    pub const SHADOW: Color =
        Color::rgba(0.0, 0.0, 0.0, 0.35);

    pub const SHADOW_SOFT: Color =
        Color::rgba(0.0, 0.0, 0.0, 0.20);

    pub const SHADOW_OFFSET_X: f32 = 0.0;
    pub const SHADOW_OFFSET_Y: f32 = 8.0;
    pub const SHADOW_RADIUS: f32 = 24.0;

    // ------------------------------------------------------------
    // Text
    // ------------------------------------------------------------

    pub const TEXT: Color =
        Color::rgba(0.94, 0.97, 1.0, 1.0);

    pub const TEXT_MUTED: Color =
        Color::rgba(0.65, 0.70, 0.78, 1.0);

    // ------------------------------------------------------------
    // MITOS accent
    // ------------------------------------------------------------

    pub const ACCENT: Color =
        Color::rgba(0.30, 0.65, 1.0, 1.0);

    // ------------------------------------------------------------
    // Geometry
    // ------------------------------------------------------------

    pub const WINDOW_RADIUS: f32 = 14.0;

    pub const PANEL_RADIUS: f32 = 18.0;

    pub const TOP_BAR_HEIGHT: f32 = 38.0;

    pub const SPACING: f32 = 8.0;

    // ------------------------------------------------------------
    // Animation
    // ------------------------------------------------------------

    pub const ANIMATION_MS: u64 = 180;
}
