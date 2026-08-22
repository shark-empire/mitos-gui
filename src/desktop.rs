//! The MITOS home screen: wallpaper and (eventually) shell chrome.
//!
//! Stage 3 starts here with the one thing every real desktop lets you
//! change without a rebuild: what the empty desktop looks like. Glass
//! panels and the top bar (also Stage 3) build on the same
//! `HomeScreenConfig` once they land -- for now this only covers the
//! wallpaper/background color.
//!
//! Config lives at `$XDG_CONFIG_HOME/mitos/home.conf` (falling back to
//! `~/.config/mitos/home.conf`), in plain `key = value` lines:
//!
//! ```text
//! # ~/.config/mitos/home.conf
//! background = #0a1420
//! ```
//!
//! A missing file, a missing `$HOME`, or a line MITOS doesn't
//! understand all fall back to the built-in theme default rather than
//! failing startup -- a broken config should never be the reason the
//! compositor won't come up.

use std::fs;
use std::path::PathBuf;

use crate::theme::{Color, MitosTheme};

#[derive(Clone, Copy, Debug)]
pub struct HomeScreenConfig {
    pub background: Color,
}

impl Default for HomeScreenConfig {
    fn default() -> Self {
        Self {
            background: MitosTheme::BACKGROUND,
        }
    }
}

impl HomeScreenConfig {
    /// Loads the home screen config from disk, falling back to
    /// [`MitosTheme`] defaults for anything missing or unreadable.
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
                "background" => match parse_hex_color(value) {
                    Some(color) => config.background = color,
                    None => tracing::warn!(
                        "MITOS GUI: {}:{}: invalid color {value:?}, keeping default",
                        path.display(),
                        line_no + 1,
                    ),
                },
                other => tracing::warn!(
                    "MITOS GUI: {}:{}: unknown key {other:?}, ignoring",
                    path.display(),
                    line_no + 1,
                ),
            }
        }

        println!(
            "MITOS GUI: home screen config loaded from {}",
            path.display()
        );

        config
    }
}

/// Resolves the home screen config path: `$XDG_CONFIG_HOME/mitos/home.conf`,
/// falling back to `$HOME/.config/mitos/home.conf`.
fn config_path() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("mitos/home.conf"));
        }
    }

    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".config/mitos/home.conf"))
}

/// Parses `#RGB`, `#RRGGBB`, or `#RRGGBBAA` into a [`Color`].
///
/// Alpha defaults to fully opaque when not specified.
fn parse_hex_color(value: &str) -> Option<Color> {
    let hex = value.strip_prefix('#')?;

    // Reject non-ASCII up front so every byte index below lands on a
    // char boundary -- otherwise a stray multibyte character in a
    // malformed config value could panic the slicing below instead of
    // just failing to parse.
    if !hex.is_ascii() {
        return None;
    }

    let expand = |byte: u8| -> Option<u8> {
        let d = (byte as char).to_digit(16)? as u8;
        Some(d * 16 + d)
    };

    let channel = |s: &str| -> Option<u8> { u8::from_str_radix(s, 16).ok() };

    let (r, g, b, a) = match hex.len() {
        3 => {
            let bytes = hex.as_bytes();
            (expand(bytes[0])?, expand(bytes[1])?, expand(bytes[2])?, 255)
        }
        6 => (
            channel(&hex[0..2])?,
            channel(&hex[2..4])?,
            channel(&hex[4..6])?,
            255,
        ),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shorthand_hex() {
        let c = parse_hex_color("#0a1420").unwrap();
        assert!((c.r - 10.0 / 255.0).abs() < f32::EPSILON);
        assert!((c.a - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parses_three_digit_hex() {
        let c = parse_hex_color("#0af").unwrap();
        assert!((c.r - 0x00 as f32 / 255.0).abs() < f32::EPSILON);
        assert!((c.g - 0xaa as f32 / 255.0).abs() < f32::EPSILON);
        assert!((c.b - 0xff as f32 / 255.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parses_alpha_channel() {
        let c = parse_hex_color("#0a142080").unwrap();
        assert!((c.a - 0x80 as f32 / 255.0).abs() < f32::EPSILON);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_hex_color("not-a-color").is_none());
        assert!(parse_hex_color("#12").is_none());
        assert!(parse_hex_color("#zzzzzz").is_none());
        assert!(parse_hex_color("#a€bcde").is_none());
    }
}
