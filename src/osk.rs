//! Stage 7: On-screen keyboard.
//!
//! Two delivery paths, with different confidence levels -- stated
//! plainly rather than glossed over:
//!
//!   1. **MITOS's own text fields** (the lock/elevation password field,
//!      the launcher search box): a click is turned directly into a
//!      `char` and handed to the exact same code path a physical
//!      keypress already goes through (`keyboard::type_char_into_*`).
//!      Full confidence -- this is MITOS's own input handling, already
//!      proven by the physical-keyboard path.
//!
//!   2. **A focused client window** (a text editor, a browser): there's
//!      no MITOS-side text field to hand a `char` to, so this
//!      synthesizes a real key event instead, using the standard Linux
//!      evdev keycode numbering (`KEY_A` = 30, etc. -- stable across
//!      the entire history of Linux input, unlike any one crate's API)
//!      run through `keyboard::dispatch_synthetic_key`. This is the
//!      lower-confidence path: it depends on `smithay::backend::input
//!      ::Keycode` having a `From<u32>` built from the evdev code, which
//!      isn't exercised anywhere else in this codebase to cross-check
//!      against. If key clicks type the wrong character or nothing when
//!      a client window is focused, this is the first place to check --
//!      the password-field/launcher path (1) is unaffected either way.
//!
//! Letters, space, backspace, enter, and shift only for now -- not a
//! full keyboard (no numbers/symbols row, no layout switching). Wide
//! enough to actually type a password or a search query, which is the
//! most common reason to need this at all; extending the layout later
//! is just more entries in `LAYOUT`, not a design change.

use smithay::utils::{Logical, Rectangle, Size};

/// One key: its label, and the evdev keycode (`linux/input-event-
/// codes.h`, not X11/xkb numbering -- callers add the standard +8
/// offset) it corresponds to, if it's a plain typeable key at all.
pub struct OskKey {
    pub label: &'static str,
    pub evdev_code: Option<u32>,
    /// Relative width vs. a standard key (1.0), for layout purposes.
    pub weight: f32,
    pub action: OskAction,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OskAction {
    /// Types `evdev_code`'s letter (respecting current shift/caps state).
    Letter,
    Space,
    Backspace,
    Enter,
    /// Toggles the shift layer for the *next* key only (not a lock).
    Shift,
}

const KEY_A: u32 = 30;
const KEY_B: u32 = 48;
const KEY_C: u32 = 46;
const KEY_D: u32 = 32;
const KEY_E: u32 = 18;
const KEY_F: u32 = 33;
const KEY_G: u32 = 34;
const KEY_H: u32 = 35;
const KEY_I: u32 = 23;
const KEY_J: u32 = 36;
const KEY_K: u32 = 37;
const KEY_L: u32 = 38;
const KEY_M: u32 = 50;
const KEY_N: u32 = 49;
const KEY_O: u32 = 24;
const KEY_P: u32 = 25;
const KEY_Q: u32 = 16;
const KEY_R: u32 = 19;
const KEY_S: u32 = 31;
const KEY_T: u32 = 20;
const KEY_U: u32 = 22;
const KEY_V: u32 = 47;
const KEY_W: u32 = 17;
const KEY_X: u32 = 45;
const KEY_Y: u32 = 21;
const KEY_Z: u32 = 44;

fn letter(label: &'static str, code: u32) -> OskKey {
    OskKey { label, evdev_code: Some(code), weight: 1.0, action: OskAction::Letter }
}

/// Three letter rows plus a bottom control row -- see the module doc
/// for why numbers/symbols aren't included yet.
pub fn layout() -> Vec<Vec<OskKey>> {
    vec![
        vec![
            letter("Q", KEY_Q), letter("W", KEY_W), letter("E", KEY_E), letter("R", KEY_R),
            letter("T", KEY_T), letter("Y", KEY_Y), letter("U", KEY_U), letter("I", KEY_I),
            letter("O", KEY_O), letter("P", KEY_P),
        ],
        vec![
            letter("A", KEY_A), letter("S", KEY_S), letter("D", KEY_D), letter("F", KEY_F),
            letter("G", KEY_G), letter("H", KEY_H), letter("J", KEY_J), letter("K", KEY_K),
            letter("L", KEY_L),
        ],
        vec![
            OskKey { label: "\u{21e7}", evdev_code: None, weight: 1.6, action: OskAction::Shift },
            letter("Z", KEY_Z), letter("X", KEY_X), letter("C", KEY_C), letter("V", KEY_V),
            letter("B", KEY_B), letter("N", KEY_N), letter("M", KEY_M),
            OskKey { label: "\u{232b}", evdev_code: None, weight: 1.6, action: OskAction::Backspace },
        ],
        vec![
            OskKey { label: "space", evdev_code: None, weight: 5.0, action: OskAction::Space },
            OskKey { label: "return", evdev_code: None, weight: 2.0, action: OskAction::Enter },
        ],
    ]
}

/// Which character a letter key types, respecting `shift`/`caps`.
pub fn key_char(key: &OskKey, shift_active: bool) -> Option<char> {
    let c = key.label.chars().next()?;
    if shift_active {
        Some(c.to_ascii_uppercase())
    } else {
        Some(c.to_ascii_lowercase())
    }
}

/// Panel and per-key geometry for the given output size. Shared by the
/// renderer (to draw) and pointer hit-testing (to know what was
/// clicked), so both always agree on where each key actually is --
/// the earlier dead-code pass found more than one bug in this
/// codebase from geometry computed twice, slightly differently, in
/// two places.
pub fn compute_geometry(
    output_size: Size<i32, Logical>,
) -> (Rectangle<i32, Logical>, Vec<Vec<Rectangle<i32, Logical>>>) {
    let rows = layout();

    let key_h = 46;
    let gap = 6;
    let panel_w = ((output_size.w as f32 * 0.7).min(760.0).max(360.0)) as i32;
    let panel_h = rows.len() as i32 * (key_h + gap) + gap + 16;
    let panel_x = (output_size.w - panel_w) / 2;
    let panel_y = output_size.h - panel_h - 24;

    let panel = Rectangle::new((panel_x, panel_y).into(), (panel_w, panel_h).into());

    let inner_w = panel_w - gap * 2;
    let mut row_rects = Vec::new();

    for (ri, row) in rows.iter().enumerate() {
        let total_weight: f32 = row.iter().map(|k| k.weight).sum();
        let unit_w = inner_w as f32 / total_weight.max(0.01);

        let mut x = panel_x + gap;
        let y = panel_y + 8 + ri as i32 * (key_h + gap);

        let mut this_row = Vec::new();
        for key in row {
            let w = ((unit_w * key.weight) as i32 - gap).max(1);
            this_row.push(Rectangle::new((x, y).into(), (w, key_h).into()));
            x += w + gap;
        }
        row_rects.push(this_row);
    }

    (panel, row_rects)
}
