//! Minimal multilingual string lookup for MITOS's own UI text (the lock
//! screen, dock labels, and anything else that used to be a hardcoded
//! English literal). Wires up the `locales/*.json` files that were
//! already sitting in the project unused.
//!
//! Deliberately small and dependency-free:
//! - No `serde_json` (not already a dependency, and pulling one in just
//!   to parse a flat `{"key": "value"}` map would be a lot of new
//!   surface for very little): `parse_flat_json` below hand-parses that
//!   one shape only -- flat string keys/values, standard escapes, no
//!   nesting, no numbers/arrays. It is NOT a general JSON parser.
//! - Locale files are embedded with `include_str!` rather than read
//!   from disk at runtime, so this never depends on the process's
//!   current working directory or an install path being right --
//!   whatever locale data shipped in the binary is what's available.
//!
//! Adding a new locale: drop `locales/<name>.json` next to the existing
//! ones, add an `include_str!` constant for it below, and add one arm
//! to `locale_source`.

use std::collections::HashMap;
use std::sync::OnceLock;

const LOCALE_EN_US: &str = include_str!("../locales/en-US.json");
const LOCALE_ES_ES: &str = include_str!("../locales/es-ES.json");

fn locale_source(name: &str) -> Option<&'static str> {
    match name {
        "en-US" => Some(LOCALE_EN_US),
        "es-ES" => Some(LOCALE_ES_ES),
        _ => None,
    }
}

static STRINGS: OnceLock<HashMap<String, String>> = OnceLock::new();
static FALLBACK: OnceLock<HashMap<String, String>> = OnceLock::new();

/// Look up a UI string by key in the detected system locale, falling
/// back to `en-US` if that locale isn't shipped or is missing this
/// specific key, and finally to the key itself -- so a missing
/// translation shows up as an obviously-wrong string instead of a
/// blank space or a panic.
pub fn t(key: &str) -> String {
    let strings = STRINGS.get_or_init(|| {
        let locale = detect_locale();
        let src = locale_source(&locale).unwrap_or(LOCALE_EN_US);
        parse_flat_json(src).into_iter().collect()
    });

    if let Some(v) = strings.get(key) {
        return v.clone();
    }

    let fallback = FALLBACK.get_or_init(|| parse_flat_json(LOCALE_EN_US).into_iter().collect());
    if let Some(v) = fallback.get(key) {
        return v.clone();
    }

    key.to_string()
}

/// Reads `$LC_ALL`, then `$LC_MESSAGES`, then `$LANG` (the standard
/// POSIX precedence) and turns e.g. `es_ES.UTF-8` into `es-ES` --
/// locale filenames use a hyphen, POSIX env vars use an underscore plus
/// often a trailing encoding this doesn't care about.
fn detect_locale() -> String {
    let raw = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default();

    let base = raw.split('.').next().unwrap_or("").replace('_', "-");
    if base.is_empty() {
        "en-US".to_string()
    } else {
        base
    }
}

/// Parses a flat `{"key": "value", ...}` JSON object into pairs. See
/// the module doc comment for why this exists instead of a real JSON
/// crate -- it only needs to understand that one shape.
fn parse_flat_json(src: &str) -> Vec<(String, String)> {
    let mut chars = src.char_indices().peekable();
    let mut pairs = Vec::new();

    loop {
        let Some(key) = read_json_string(&mut chars) else { break };
        let Some(value) = read_json_string(&mut chars) else { break };
        pairs.push((key, value));
    }

    pairs
}

/// Advances past the next `"..."` JSON string literal (skipping
/// whatever structural characters -- `{`, `}`, `:`, `,`, whitespace --
/// come before it) and returns its decoded contents. Returns `None`
/// once there are no more string literals left.
fn read_json_string(chars: &mut std::iter::Peekable<std::str::CharIndices>) -> Option<String> {
    loop {
        match chars.next()? {
            (_, '"') => break,
            _ => continue,
        }
    }

    let mut out = String::new();
    loop {
        match chars.next()? {
            (_, '\\') => match chars.next()? {
                (_, 'n') => out.push('\n'),
                (_, 't') => out.push('\t'),
                (_, 'r') => out.push('\r'),
                (_, c) => out.push(c), // covers `\"` and `\\` too
            },
            (_, '"') => break,
            (_, c) => out.push(c),
        }
    }

    Some(out)
}
