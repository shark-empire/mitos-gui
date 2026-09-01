//! MITOS desktop and shell layout.
//!
//! This module owns the logical desktop configuration and shell geometry.
//!
//! Stage 3 responsibilities:
//! - desktop background configuration
//! - top-bar configuration
//! - launcher configuration
//! - dock configuration
//!
//! Rendering remains in `renderer.rs`.
//! This module deliberately does not contain GLES/OpenGL code.

use std::fs;
use std::path::PathBuf;

use smithay::{
    backend::renderer::Color32F,
    utils::Size,
};

use crate::renderer::GlassPanel;
use crate::theme::{Color, MitosTheme};

// ============================================================================
// HOME SCREEN CONFIGURATION
// ============================================================================

/// Configuration for the MITOS home screen.
#[derive(Clone, Debug)]
pub struct HomeScreenConfig {
    /// Desktop background color.
    pub background: Color,

    /// Whether the top bar is visible.
    pub top_bar: bool,

    /// Top bar height in logical pixels.
    pub top_bar_height: f32,

    /// Whether the launcher is available.
    pub launcher: bool,

    /// Whether the dock is visible.
    pub dock: bool,

    /// Dock height in logical pixels.
    pub dock_height: f32,

    /// Launcher width in logical pixels.
    pub launcher_width: f32,

    /// Launcher height in logical pixels.
    pub launcher_height: f32,

    /// "light" or "dark".
    pub theme_mode: String,

    /// Optional accent override.
    pub accent_color: Option<Color>,

    /// Optional glass opacity override (0.0..=1.0).
    pub glass_opacity: Option<f32>,

    /// Optional panel radius override.
    pub panel_radius: Option<f32>,

    /// Optional wallpaper path override.
    pub wallpaper_path: Option<String>,

    /// Whether Night Light (blue light filter) is enabled.
    pub night_light: bool,
}

impl Default for HomeScreenConfig {
    fn default() -> Self {
        Self {
            background: MitosTheme::BACKGROUND,

            top_bar: true,
            top_bar_height: MitosTheme::TOP_BAR_HEIGHT,

            launcher: true,

            dock: true,
            dock_height: 72.0,

            launcher_width: 720.0,
            launcher_height: 520.0,

            theme_mode: "dark".to_string(),
            accent_color: None,
            glass_opacity: None,
            panel_radius: None,
            wallpaper_path: None,
            night_light: false,
        }
    }
}

impl HomeScreenConfig {
    /// Load the MITOS home-screen configuration.
    ///
    /// Missing or malformed configuration never prevents MITOS from
    /// starting. Invalid values simply retain their defaults.
    pub fn load() -> Self {
        let mut config = Self::default();

        let Some(path) = config_path() else {
            println!("MITOS GUI: no $HOME, using default home screen config");
            return config;
        };

        let Ok(contents) = fs::read_to_string(&path) else {
            println!(
                "MITOS GUI: no home screen config at {}, using defaults",
                path.display()
            );
            return config;
        };

        let mut saw_background = false;

        for (line_no, line) in contents.lines().enumerate() {
            let line = line.trim();

            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let Some((key, value)) = line.split_once('=') else {
                tracing::warn!(
                    "MITOS GUI: {}:{}: expected `key = value`, got {line:?}",
                    path.display(),
                    line_no + 1,
                );
                continue;
            };

            let key = key.trim();
            let value = value.trim();

            match key {
                // --------------------------------------------------------
                // Background
                // --------------------------------------------------------

                "background" => match parse_hex_color(value) {
                    Some(color) => {
                        config.background = color;
                        saw_background = true;
                    }
                    None => tracing::warn!(
                        "MITOS GUI: {}:{}: invalid color {value:?}, keeping default",
                        path.display(),
                        line_no + 1,
                    ),
                },

                // --------------------------------------------------------
                // Top bar
                // --------------------------------------------------------

                "top_bar" => match parse_bool(value) {
                    Some(enabled) => config.top_bar = enabled,
                    None => tracing::warn!(
                        "MITOS GUI: {}:{}: expected `true` or `false`, got {value:?}, keeping default",
                        path.display(),
                        line_no + 1,
                    ),
                },

                "top_bar_height" => match parse_positive_f32(value) {
                    Some(height) => config.top_bar_height = height,
                    None => tracing::warn!(
                        "MITOS GUI: {}:{}: invalid top bar height {value:?}, keeping default",
                        path.display(),
                        line_no + 1,
                    ),
                },

                // --------------------------------------------------------
                // Launcher
                // --------------------------------------------------------

                "launcher" => match parse_bool(value) {
                    Some(enabled) => config.launcher = enabled,
                    None => tracing::warn!(
                        "MITOS GUI: {}:{}: expected `true` or `false`, got {value:?}, keeping default",
                        path.display(),
                        line_no + 1,
                    ),
                },

                "launcher_width" => match parse_positive_f32(value) {
                    Some(width) => config.launcher_width = width,
                    None => tracing::warn!(
                        "MITOS GUI: {}:{}: invalid launcher width {value:?}, keeping default",
                        path.display(),
                        line_no + 1,
                    ),
                },

                "launcher_height" => match parse_positive_f32(value) {
                    Some(height) => config.launcher_height = height,
                    None => tracing::warn!(
                        "MITOS GUI: {}:{}: invalid launcher height {value:?}, keeping default",
                        path.display(),
                        line_no + 1,
                    ),
                },

                // --------------------------------------------------------
                // Dock
                // --------------------------------------------------------

                "dock" => match parse_bool(value) {
                    Some(enabled) => config.dock = enabled,
                    None => tracing::warn!(
                        "MITOS GUI: {}:{}: expected `true` or `false`, got {value:?}, keeping default",
                        path.display(),
                        line_no + 1,
                    ),
                },

                "dock_height" => match parse_positive_f32(value) {
                    Some(height) => config.dock_height = height,
                    None => tracing::warn!(
                        "MITOS GUI: {}:{}: invalid dock height {value:?}, keeping default",
                        path.display(),
                        line_no + 1,
                    ),
                },

                // --------------------------------------------------------
                // Shared MITOS Config (Theme & Live Reload)
                // --------------------------------------------------------

                "theme_mode" => {
                    if value == "light" || value == "dark" {
                        config.theme_mode = value.to_string();
                    } else {
                        tracing::warn!(
                            "MITOS GUI: {}:{}: invalid theme_mode {value:?}, expected `light` or `dark`",
                            path.display(),
                            line_no + 1,
                        );
                    }
                }

                "accent_color" => match parse_hex_color(value) {
                    Some(color) => config.accent_color = Some(color),
                    None => tracing::warn!(
                        "MITOS GUI: {}:{}: invalid accent color {value:?}",
                        path.display(),
                        line_no + 1,
                    ),
                },

                "glass_opacity" => match value.parse::<f32>() {
                    Ok(v) if v.is_finite() && (0.0..=1.0).contains(&v) => {
                        config.glass_opacity = Some(v);
                    }
                    _ => tracing::warn!(
                        "MITOS GUI: {}:{}: invalid glass opacity {value:?}",
                        path.display(),
                        line_no + 1,
                    ),
                },

                "panel_radius" => match parse_positive_f32(value) {
                    Some(v) => config.panel_radius = Some(v),
                    None => tracing::warn!(
                        "MITOS GUI: {}:{}: invalid panel radius {value:?}",
                        path.display(),
                        line_no + 1,
                    ),
                },

                "wallpaper" => {
                    if !value.is_empty() {
                        config.wallpaper_path = Some(value.to_string());
                    }
                }

                "night_light" => match parse_bool(value) {
                    Some(enabled) => config.night_light = enabled,
                    None => tracing::warn!(
                        "MITOS GUI: {}:{}: expected `true` or `false` for night_light, got {value:?}",
                        path.display(),
                        line_no + 1,
                    ),
                },

                // --------------------------------------------------------
                // Unknown
                // --------------------------------------------------------

                other => tracing::warn!(
                    "MITOS GUI: {}:{}: unknown key {other:?}, ignoring",
                    path.display(),
                    line_no + 1,
                ),
            }
        }

        // Light mode without an explicit background gets a light desktop.
        if config.theme_mode == "light" && !saw_background {
            config.background = Color::rgba(0.96, 0.97, 0.98, 1.0);
        }

        println!(
            "MITOS GUI: home screen config loaded from {}",
            path.display()
        );

        config
    }
}

// ============================================================================
// SHELL LAYOUT
// ============================================================================

/// Logical geometry of the MITOS shell.
///
/// This is deliberately independent from GLES.
///
/// `desktop.rs` calculates the geometry.
/// `renderer.rs` renders it.
#[derive(Clone, Copy, Debug)]
pub struct ShellLayout {
    /// Top bar panel.
    pub top_bar: Option<GlassPanel>,

    /// Application launcher panel.
    pub launcher: Option<GlassPanel>,

    /// Bottom dock panel.
    pub dock: Option<GlassPanel>,
}

impl ShellLayout {
    /// Calculate the shell layout for the current output.
    pub fn calculate(
        config: &HomeScreenConfig,
        output_size: Size<i32, smithay::utils::Logical>,
    ) -> Self {
        let width = output_size.w.max(1);
        let height = output_size.h.max(1);

        // ------------------------------------------------------------
        // Top bar
        // ------------------------------------------------------------

        let top_bar = if config.top_bar {
            let top_bar_height = config.top_bar_height.max(1.0).round() as i32;

            Some(GlassPanel::top_bar(width, top_bar_height))
        } else {
            None
        };

        // ------------------------------------------------------------
        // Launcher
        //
        // Centered horizontally and vertically.
        // ------------------------------------------------------------

        let launcher = if config.launcher {
            let launcher_width = config
                .launcher_width
                .min(width as f32)
                .max(1.0)
                .round() as i32;

            let launcher_height = config
                .launcher_height
                .min(height as f32)
                .max(1.0)
                .round() as i32;

            let x = ((width - launcher_width) / 2).max(0);
            let y = ((height - launcher_height) / 2).max(0);

            Some(GlassPanel {
                position: (x, y),
                size: (launcher_width, launcher_height),
                radius: MitosTheme::effective_panel_radius(),
                tint: crate::renderer::glass_color(),
                border: Color32F::new(
                    MitosTheme::BORDER.r,
                    MitosTheme::BORDER.g,
                    MitosTheme::BORDER.b,
                    MitosTheme::BORDER.a,
                ),
            })
        } else {
            None
        };

        // ------------------------------------------------------------
        // Dock
        //
        // Floating glass dock centered horizontally near the bottom.
        // ------------------------------------------------------------

        let dock = if config.dock {
            let dock_height = config
                .dock_height
                .max(64.0)
                .min(height as f32)
                .round() as i32;

            let dock_width = ((width as f32) * 0.55)
                .max(360.0)
                .min(width as f32 - 32.0)
                .round() as i32;

            let bottom_margin = 20;

            let x = ((width - dock_width) / 2).max(0);
            let y = (height - dock_height - bottom_margin).max(0);

            Some(GlassPanel {
                position: (x, y),
                size: (dock_width, dock_height),
                radius: MitosTheme::effective_panel_radius(),
                tint: crate::renderer::glass_color(),
                border: Color32F::new(
                    MitosTheme::BORDER.r,
                    MitosTheme::BORDER.g,
                    MitosTheme::BORDER.b,
                    MitosTheme::BORDER.a,
                ),
            })
        } else {
            None
        };

        Self {
            top_bar,
            launcher,
            dock,
        }
    }
}

// ============================================================================
// DOCK
// ============================================================================

/// An application represented by a dock icon.
#[derive(Clone, Debug)]
pub struct DockItem {
    /// Stable identifier used by input handling.
    pub id: &'static str,

    /// Human-readable application name.
    pub name: &'static str,

    /// Whether this application is currently running.
    pub running: bool,

    /// Whether this item is currently selected.
    pub active: bool,
}

/// Layout information for the MITOS dock.
#[derive(Clone, Debug)]
pub struct DockLayout {
    /// Applications displayed in the dock.
    pub items: Vec<DockItem>,

    /// Size of each icon.
    pub icon_size: i32,

    /// Space between icons.
    pub spacing: i32,

    /// Horizontal padding inside the glass panel.
    pub padding: i32,
}

impl Default for DockLayout {
    fn default() -> Self {
        Self {
            items: vec![
                DockItem {
                    id: "launcher",
                    name: "Launcher",
                    running: false,
                    active: false,
                },
                DockItem {
                    id: "files",
                    name: "Files",
                    running: false,
                    active: false,
                },
                DockItem {
                    id: "terminal",
                    name: "Terminal",
                    running: false,
                    active: false,
                },
                DockItem {
                    id: "browser",
                    name: "Browser",
                    running: false,
                    active: false,
                },
                DockItem {
                    id: "settings",
                    name: "Settings",
                    running: false,
                    active: false,
                },
            ],

            icon_size: 44,
            spacing: 12,
            padding: 18,
        }
    }
}

// ============================================================================
// CONFIGURATION PATH
// ============================================================================

/// Resolve the MITOS home configuration path.
///
/// Priority:
///
/// 1. `$XDG_CONFIG_HOME/mitos/home.conf`
/// 2. `$HOME/.config/mitos/home.conf`
fn config_path() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(
                PathBuf::from(xdg)
                    .join("mitos")
                    .join("home.conf"),
            );
        }
    }

    std::env::var("HOME")
        .ok()
        .map(|home| {
            PathBuf::from(home)
                .join(".config")
                .join("mitos")
                .join("home.conf")
        })
}

// ============================================================================
// PARSERS
// ============================================================================

/// Parse `true` or `false`.
fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Parse a positive floating-point value.
fn parse_positive_f32(value: &str) -> Option<f32> {
    let number = value.parse::<f32>().ok()?;

    if number.is_finite() && number > 0.0 {
        Some(number)
    } else {
        None
    }
}

/// Parse `#RGB`, `#RRGGBB`, or `#RRGGBBAA`.
///
/// Alpha defaults to fully opaque when omitted.
fn parse_hex_color(value: &str) -> Option<Color> {
    let hex = value.strip_prefix('#')?;

    if !hex.is_ascii() {
        return None;
    }

    let expand = |byte: u8| -> Option<u8> {
        let digit = (byte as char).to_digit(16)? as u8;
        Some(digit * 16 + digit)
    };

    let channel = |value: &str| -> Option<u8> {
        u8::from_str_radix(value, 16).ok()
    };

    let (r, g, b, a) = match hex.len() {
        // #RGB
        3 => {
            let bytes = hex.as_bytes();
            (
                expand(bytes[0])?,
                expand(bytes[1])?,
                expand(bytes[2])?,
                255,
            )
        }

        // #RRGGBB
        6 => (
            channel(&hex[0..2])?,
            channel(&hex[2..4])?,
            channel(&hex[4..6])?,
            255,
        ),

        // #RRGGBBAA
        8 => (
            channel(&hex[0..2])?,
            channel(&hex[2..4])?,
            channel(&hex[4..6])?,
            channel(&hex[6..8])?,
        ),

        _ => return None,
    };

    Some(Color::rgba(
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ))
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_three_digit_hex() {
        let color = parse_hex_color("#0af").unwrap();

        assert!((color.r - 0.0).abs() < f32::EPSILON);
        assert!((color.g - 170.0 / 255.0).abs() < f32::EPSILON);
        assert!((color.b - 255.0 / 255.0).abs() < f32::EPSILON);
        assert!((color.a - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parses_six_digit_hex() {
        let color = parse_hex_color("#0a1420").unwrap();

        assert!((color.r - 10.0 / 255.0).abs() < f32::EPSILON);
        assert!((color.g - 20.0 / 255.0).abs() < f32::EPSILON);
        assert!((color.b - 32.0 / 255.0).abs() < f32::EPSILON);
        assert!((color.a - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parses_alpha_channel() {
        let color = parse_hex_color("#0a142080").unwrap();

        assert!((color.a - 128.0 / 255.0).abs() < f32::EPSILON);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_hex_color("not-a-color").is_none());
        assert!(parse_hex_color("#12").is_none());
        assert!(parse_hex_color("#zzzzzz").is_none());
        assert!(parse_hex_color("#a€bcde").is_none());
    }

    #[test]
    fn parses_bool() {
        assert_eq!(parse_bool("true"), Some(true));
        assert_eq!(parse_bool("false"), Some(false));
        assert_eq!(parse_bool("yes"), None);
        assert_eq!(parse_bool("1"), None);
    }

    #[test]
    fn parses_positive_number() {
        assert_eq!(parse_positive_f32("32"), Some(32.0));
        assert_eq!(parse_positive_f32("72.5"), Some(72.5));
        assert_eq!(parse_positive_f32("0"), None);
        assert_eq!(parse_positive_f32("-1"), None);
    }
}
