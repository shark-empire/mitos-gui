//! Stage 7 (Phase 3): Quick Settings panel.
//!
//! A dropdown from the top bar's tray/clock area -- Super+S or a click
//! there toggles it. Built entirely on state this codebase already
//! tracks (volume, brightness, night light, dark/light mode, network,
//! battery, plus Bluetooth and the power profile, which come from
//! mitos-bluetooth and mitos-power via `services.rs`); the one new piece
//! of state is Do Not Disturb, which lives
//! on `NotificationManager::dnd` rather than here (see `notifications
//! .rs`) since that's the type that actually needs to check it.
//!
//! Sliders (volume, brightness) are click-to-set, not drag: a click at
//! some position along the bar sets the value proportionally, same
//! single-click model as everything else clickable in this codebase.
//! Dragging to scrub would need pointer-motion tracking this module
//! doesn't have wired to it, which is a reasonable next step rather
//! than a gap this pass silently papers over.

use smithay::utils::{Logical, Rectangle, Size};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QsControl {
    VolumeSlider,
    MuteToggle,
    BrightnessSlider,
    NightLightToggle,
    DarkModeToggle,
    DndToggle,
    /// mitos-bluetooth: adapter on/off (`services.rs`).
    BluetoothToggle,
    /// mitos-power: cycle to the next power profile (`services.rs`).
    PowerProfileCycle,
    SettingsButton,
}

pub const PANEL_WIDTH: i32 = 300;
const MARGIN: i32 = 12;
const ROW_H: i32 = 40;
const SLIDER_H: i32 = 14;
const TOGGLE_H: i32 = 36;
const GAP: i32 = 10;
/// One caption line under the Bluetooth/power-profile row (connected
/// devices and their battery levels).
const CAPTION_H: i32 = 18;

/// Panel and per-control geometry for the given output size and top
/// bar height. Shared by the renderer (to draw) and pointer
/// hit-testing (to know what was clicked/where along a slider), so
/// both always agree on where each control actually is -- same reason
/// `osk::compute_geometry` and `wm::usable_area` exist as shared
/// functions rather than being computed twice.
pub fn compute_geometry(
    output_size: Size<i32, Logical>,
    top_bar_height: i32,
) -> (Rectangle<i32, Logical>, Vec<(QsControl, Rectangle<i32, Logical>)>) {
    let panel_x = output_size.w - PANEL_WIDTH - MARGIN;
    let panel_y = top_bar_height + MARGIN;

    // Header text + network/battery info lines + volume row + brightness
    // row + toggle row + settings row, each ROW_H apart, plus margins.
    let panel_h = MARGIN + ROW_H /* header */ + ROW_H /* info */
        + ROW_H /* volume */ + ROW_H /* brightness */
        + TOGGLE_H + GAP /* toggles */
        + TOGGLE_H + CAPTION_H + GAP /* bluetooth + power profile, caption */
        + ROW_H /* settings button */
        + MARGIN;

    let panel = Rectangle::new((panel_x, panel_y).into(), (PANEL_WIDTH, panel_h).into());

    let inner_x = panel_x + MARGIN;
    let inner_w = PANEL_WIDTH - MARGIN * 2;
    let mut y = panel_y + MARGIN + ROW_H + ROW_H; // past header + info line

    let mut controls = Vec::new();

    // Volume: label above, slider below, mute toggle button at the
    // slider's right end (a small fixed-width square cut from inner_w).
    let mute_w = 28;
    let vol_slider_w = inner_w - mute_w - GAP;
    controls.push((
        QsControl::VolumeSlider,
        Rectangle::new(
            (inner_x, y + ROW_H - SLIDER_H - 4).into(),
            (vol_slider_w, SLIDER_H).into(),
        ),
    ));
    controls.push((
        QsControl::MuteToggle,
        Rectangle::new(
            (inner_x + vol_slider_w + GAP, y + ROW_H - SLIDER_H - 4 - 7).into(),
            (mute_w, 28).into(),
        ),
    ));
    y += ROW_H;

    // Brightness: same shape, full width (no toggle button needed).
    controls.push((
        QsControl::BrightnessSlider,
        Rectangle::new((inner_x, y + ROW_H - SLIDER_H - 4).into(), (inner_w, SLIDER_H).into()),
    ));
    y += ROW_H;

    // Three toggle pills in a row.
    let toggle_gap = GAP;
    let toggle_w = (inner_w - toggle_gap * 2) / 3;
    for (i, control) in [QsControl::NightLightToggle, QsControl::DarkModeToggle, QsControl::DndToggle]
        .into_iter()
        .enumerate()
    {
        let x = inner_x + i as i32 * (toggle_w + toggle_gap);
        controls.push((control, Rectangle::new((x, y).into(), (toggle_w, TOGGLE_H).into())));
    }
    y += TOGGLE_H + GAP;

    // Second row: Bluetooth and the power profile, two wider pills, with a
    // caption line below (drawn by the renderer, not a control).
    let half_w = (inner_w - toggle_gap) / 2;
    for (i, control) in [QsControl::BluetoothToggle, QsControl::PowerProfileCycle]
        .into_iter()
        .enumerate()
    {
        let x = inner_x + i as i32 * (half_w + toggle_gap);
        controls.push((control, Rectangle::new((x, y).into(), (half_w, TOGGLE_H).into())));
    }
    y += TOGGLE_H + CAPTION_H + GAP;

    controls.push((
        QsControl::SettingsButton,
        Rectangle::new((inner_x, y).into(), (inner_w, ROW_H - 8).into()),
    ));

    (panel, controls)
}

/// Given a click at `x` within `slider` (already confirmed to be
/// inside its bounds), the 0-100 value that position represents.
pub fn slider_value_at(slider: Rectangle<i32, Logical>, x: i32) -> u8 {
    let rel = (x - slider.loc.x).max(0) as f32 / slider.size.w.max(1) as f32;
    (rel.clamp(0.0, 1.0) * 100.0).round() as u8
}
