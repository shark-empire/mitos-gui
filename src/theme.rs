//! MITOS visual design system.
//!
//! Centralized theme values keep the desktop consistent and make it
//! possible to change the appearance without touching compositor logic.

use std::sync::RwLock;

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

        // ------------------------------------------------------------
    // LIQUID GLASS
    // ------------------------------------------------------------

    /// Strength of the top light specular sweep (0.0 - 1.0).
    pub const LIQUID_SPECULAR: f32 = 0.50;

    /// Strength of the fresnel rim light at panel edges.
    pub const LIQUID_RIM: f32 = 0.45;

    /// Subtle surface grain so the glass doesn't look flat.
    pub const LIQUID_GRAIN: f32 = 0.015;

    /// How strongly dock icons magnify near the pointer (0.0 - 1.0).
    pub const DOCK_MAGNIFICATION: f32 = 0.55;

    /// Effective specular respecting runtime theme.
    pub fn effective_specular() -> f32 {
        match Self::runtime() {
            Some(rt) if !rt.dark_mode => Self::LIQUID_SPECULAR * 0.8,
            _ => Self::LIQUID_SPECULAR,
        }
    }

}

// ============================================================================
// RUNTIME THEME (live reload from home.conf)
// ============================================================================

/// Runtime theme overrides loaded from ~/.config/mitos/home.conf.
///
/// When MITOS Files writes new theme values to the shared config,
/// the compositor reloads this and updates all glass panels, accents,
/// and background colors without restarting.
#[derive(Clone, Copy, Debug)]
pub struct RuntimeTheme {
    /// Whether dark mode is active.
    pub dark_mode: bool,
    
    /// Glass panel opacity (0.0 = transparent, 1.0 = opaque).
    pub glass_alpha: f32,
    
    /// Panel corner radius in logical pixels.
    pub panel_radius: f32,
    
    /// Accent color for active elements.
    pub accent: Color,
    
    /// Glass tint color (overrides GLASS constant).
    pub glass_tint: Color,
}

/// Global runtime theme state.
///
/// `None` means use the default `MitosTheme` constants.
/// `Some(...)` means overrides are active from home.conf.
static RUNTIME_THEME: RwLock<Option<RuntimeTheme>> = RwLock::new(None);

impl MitosTheme {
    /// Apply runtime theme overrides from the loaded config.
    ///
    /// Called by `MitosGuiState::reload_configuration()` when
    /// ~/.config/mitos/home.conf changes.
    pub fn apply_runtime(config: &crate::desktop::HomeScreenConfig) {
        let is_dark = config.theme_mode != "light";
        
        let glass_alpha = config.glass_opacity.unwrap_or(Self::GLASS.a);
        
        let glass_tint = if is_dark {
            Color::rgba(0.10, 0.13, 0.18, glass_alpha)
        } else {
            Color::rgba(0.92, 0.95, 0.98, glass_alpha)
        };
        
        let rt = RuntimeTheme {
            dark_mode: is_dark,
            glass_alpha,
            panel_radius: config.panel_radius.unwrap_or(Self::PANEL_RADIUS),
            accent: config.accent_color.unwrap_or(Self::ACCENT),
            glass_tint,
        };

        *RUNTIME_THEME.write().unwrap() = Some(rt);
    }

    /// Get the current runtime theme, if any.
    pub fn runtime() -> Option<RuntimeTheme> {
        RUNTIME_THEME.read().unwrap().clone()
    }

    /// Get the effective glass color (runtime override or default).
    pub fn effective_glass() -> Color {
        Self::runtime()
            .map(|rt| rt.glass_tint)
            .unwrap_or(Self::GLASS)
    }

    /// Get the effective accent color (runtime override or default).
    pub fn effective_accent() -> Color {
        Self::runtime()
            .map(|rt| rt.accent)
            .unwrap_or(Self::ACCENT)
    }

    /// Get the effective panel radius (runtime override or default).
    pub fn effective_panel_radius() -> f32 {
        Self::runtime()
            .map(|rt| rt.panel_radius)
            .unwrap_or(Self::PANEL_RADIUS)
    }

    /// Get the effective background color.
    pub fn effective_background() -> Color {
        if let Some(rt) = Self::runtime() {
            if rt.dark_mode {
                Color::rgba(0.025, 0.035, 0.055, 1.0)
            } else {
                Color::rgba(0.96, 0.97, 0.98, 1.0)
            }
        } else {
            Self::BACKGROUND
        }
    }

    /// Check if dark mode is currently active.
    pub fn is_dark_mode() -> bool {
        Self::runtime()
            .map(|rt| rt.dark_mode)
            .unwrap_or(true) // Default to dark
    }
}
