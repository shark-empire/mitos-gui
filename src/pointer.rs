//! MITOS pointer input.
//!
//! Stage 3 & 4:
//! - track the compositor pointer position
//! - find the window underneath the pointer
//! - focus the window on button press
//! - forward pointer events to the focused Wayland surface
//! - Stage 4: window dragging (Super + left-drag)
//! - Stage 4: window resizing (Super + right-drag)
//! - Stage 7: Hot corners (Launcher & Night Light)

use std::time::{Duration, Instant};

use smithay::backend::input::{
    AbsolutePositionEvent,
    Axis,
    ButtonState,
    Event,
    InputBackend,
    PointerAxisEvent,
    PointerButtonEvent,
};

use smithay::input::pointer::{
    AxisFrame,
    ButtonEvent,
    MotionEvent,
};

use smithay::output::Output;

use smithay::utils::{
    Logical,
    Point,
    Size,
    SERIAL_COUNTER,
};

use smithay::wayland::seat::WaylandFocus;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use crate::state::MitosGuiState;

/// Linux input button codes.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

/// Handle absolute pointer motion.
///
/// Winit reports the pointer position as a normalized
/// coordinate in the range 0.0..=1.0. Convert it into
/// the logical output coordinates used by MITOS.
pub fn handle_pointer_motion_absolute<B: InputBackend>(
    state: &mut MitosGuiState,
    output: &Output,
    event: B::PointerMotionAbsoluteEvent,
) {
    let size = output
        .current_mode()
        .map(|mode| mode.size)
        .unwrap_or_else(|| (1, 1).into());

    let position = event.position();

    let x = position.x * size.w as f64;
    let y = position.y * size.h as f64;

    state.pointer_location = Point::<f64, Logical>::from((x, y));

    // --- HOT CORNERS CHECK ---
    let logical_size = Size::<i32, Logical>::new(size.w, size.h);
    check_hot_corners(state, state.pointer_location, logical_size);

    // Interactive move/resize follows the pointer.
    if state.interactive.is_some() {
        crate::wm::update_interactive(state);
    }

    let Some(pointer) = state.seat.get_pointer() else {
        return;
    };

    let serial = SERIAL_COUNTER.next_serial();

    let focus = pointer_focus(state);

    pointer.motion(
        state,
        focus,
        &MotionEvent {
            location: state.pointer_location,
            serial,
            time: event.time_msec(),
        },
    );
}

/// Handle pointer button presses/releases.
pub fn handle_pointer_button<B: InputBackend>(
    state: &mut MitosGuiState,
    event: B::PointerButtonEvent,
) {
    let Some(pointer) = state.seat.get_pointer() else {
        return;
    };

    let serial = SERIAL_COUNTER.next_serial();
    let button = event.button_code();

    let button_state = event.state();

    let under = window_under_pointer(state);

    // ------------------------------------------------------------
    // End interactive move/resize on release.
    // ------------------------------------------------------------
    if matches!(button_state, ButtonState::Released) {
        if state.interactive.is_some() {
            crate::wm::end_interactive(state);
            return;
        }
    }

    // ------------------------------------------------------------
    // Workspace overview (Stage 7, Phase 3).
    // ------------------------------------------------------------
    if let Some(output_name) = state.overview_output.clone() {
        if matches!(button_state, ButtonState::Pressed) && button == BTN_LEFT {
            let (px, py) = (state.pointer_location.x as i32, state.pointer_location.y as i32);
            let output_size = crate::overview::output_size_for(state, &output_name);
            let (panel, cells) = crate::overview::compute_geometry(output_size);

            let inside_panel = px >= panel.loc.x
                && px < panel.loc.x + panel.size.w
                && py >= panel.loc.y
                && py < panel.loc.y + panel.size.h;

            if inside_panel {
                for (i, rect) in cells.iter().enumerate() {
                    let hit = px >= rect.loc.x
                        && px < rect.loc.x + rect.size.w
                        && py >= rect.loc.y
                        && py < rect.loc.y + rect.size.h;
                    if hit {
                        state.switch_workspace(&output_name, i);
                        state.overview_output = None;
                        break;
                    }
                }
            } else {
                state.overview_output = None;
            }

            state.pending_full_redraw = true;
        }
        // Modal: swallow every button event (presses *and* releases,
        // any button) while the overview is open, so nothing leaks
        // through to the dock or a window underneath it.
        return;
    }

    // ------------------------------------------------------------
    // Quick Settings (Stage 7, Phase 3).
    // ------------------------------------------------------------
    if matches!(button_state, ButtonState::Pressed) && button == BTN_LEFT {
        let (px, py) = (state.pointer_location.x as i32, state.pointer_location.y as i32);

        if state.quick_settings_visible {
            let top_bar_h = state.shell.top_bar.map(|p| p.size.1).unwrap_or(0);
            let output_size = crate::wm::output_size(state);
            let (panel, controls) = crate::quick_settings::compute_geometry(output_size, top_bar_h);

            let inside_panel = px >= panel.loc.x
                && px < panel.loc.x + panel.size.w
                && py >= panel.loc.y
                && py < panel.loc.y + panel.size.h;

            if inside_panel {
                for (control, rect) in &controls {
                    let hit = px >= rect.loc.x
                        && px < rect.loc.x + rect.size.w
                        && py >= rect.loc.y
                        && py < rect.loc.y + rect.size.h;
                    if hit {
                        handle_quick_settings_click(state, *control, px, *rect);
                        break;
                    }
                }
            } else {
                // Clicked away -- dismiss, same as most dropdown panels.
                state.quick_settings_visible = false;
            }

            state.pending_full_redraw = true;
            return;
        }

        // Not open yet -- did this click land in the top bar's
        // tray/clock zone, where clicking is how Quick Settings opens
        // (mirrors Super+S)?
        if let Some(panel) = state.shell.top_bar.as_ref() {
            let zone_w = crate::quick_settings::PANEL_WIDTH.min(panel.size.0);
            let zone_x = panel.position.0 + panel.size.0 - zone_w;
            let inside_zone = px >= zone_x
                && px < panel.position.0 + panel.size.0
                && py >= panel.position.1
                && py < panel.position.1 + panel.size.1;

            if inside_zone {
                state.quick_settings_visible = true;
                state.pending_full_redraw = true;
                return;
            }
        }
    }

    // ------------------------------------------------------------
    // On-screen keyboard key click (Stage 7).
    // ------------------------------------------------------------
    if state.osk_visible && matches!(button_state, ButtonState::Pressed) && button == BTN_LEFT {
        let output_size = crate::wm::output_size(state);
        let (panel, row_rects) = crate::osk::compute_geometry(output_size);
        let (px, py) = (state.pointer_location.x as i32, state.pointer_location.y as i32);

        let inside_panel = px >= panel.loc.x
            && px < panel.loc.x + panel.size.w
            && py >= panel.loc.y
            && py < panel.loc.y + panel.size.h;

        if inside_panel {
            let rows = crate::osk::layout();
            'hit: for (ri, row) in row_rects.iter().enumerate() {
                for (ki, rect) in row.iter().enumerate() {
                    let hit = px >= rect.loc.x
                        && px < rect.loc.x + rect.size.w
                        && py >= rect.loc.y
                        && py < rect.loc.y + rect.size.h;
                    if hit {
                        if let Some(key) = rows.get(ri).and_then(|r| r.get(ki)) {
                            crate::keyboard::handle_osk_key(state, key);
                        }
                        break 'hit;
                    }
                }
            }
            // Landed on the OSK panel either way -- don't fall through
            // to dock/window click handling underneath it.
            return;
        }
    }

    // ------------------------------------------------------------
    // Dock icon click.
    // ------------------------------------------------------------
    if matches!(button_state, ButtonState::Pressed) && button == BTN_LEFT {
        if let Some(app_id) = dock_icon_under_pointer(state) {
            crate::shell_interaction::launch_app(state, app_id);
            return;
        }
    }

    // ------------------------------------------------------------
    // Context menu (Stage 7, Phase 3): item click or click-away.
    // Modal while open: every button event is swallowed (including
    // the release that follows the right-click which opened it), so
    // nothing leaks through to a window or the dock underneath.
    // ------------------------------------------------------------
    if state.context_menu.is_some() {
        if matches!(button_state, ButtonState::Pressed) {
            let (px, py) = (state.pointer_location.x as i32, state.pointer_location.y as i32);
            let output_size = crate::wm::output_size(state);

            if button == BTN_LEFT {
                if let Some(menu) = state.context_menu.take() {
                    let rects = menu.item_rects(output_size);
                    for (i, rect) in rects.iter().enumerate() {
                        let hit = px >= rect.loc.x
                            && px < rect.loc.x + rect.size.w
                            && py >= rect.loc.y
                            && py < rect.loc.y + rect.size.h;
                        if hit {
                            if let Some((_, action)) = menu.items.get(i).cloned() {
                                crate::context_menu::execute(state, action);
                            }
                            break;
                        }
                    }
                }
            } else {
                // Right-click (or any other button) while a menu is
                // already open just closes it, rather than opening a
                // second one.
                state.context_menu = None;
            }

            state.pending_full_redraw = true;
        }
        return;
    }

    // ------------------------------------------------------------
    // Desktop icons + background (Stage 7, Phase 3): left-click
    // selects (double-click opens), right-click opens a context menu.
    // Only on bare, visible desktop -- not over a window, the dock,
    // the top bar, or an open launcher.
    // ------------------------------------------------------------
    if matches!(button_state, ButtonState::Pressed)
        && (button == BTN_LEFT || button == BTN_RIGHT)
        && !state.shell.launcher_visible
    {
        let (px, py) = (state.pointer_location.x as i32, state.pointer_location.y as i32);

        if is_on_bare_desktop(state, px, py) && !visible_window_at(state, px, py) {
            let hit = desktop_icon_under_pointer(state, px, py);

            if button == BTN_RIGHT {
                state.desktop_icon_selected = hit;
                state.context_menu = Some(match hit {
                    Some(i) => {
                        let name = state.desktop_icons[i].name.clone();
                        crate::context_menu::ContextMenu::for_icon(px, py, &name, i)
                    }
                    None => crate::context_menu::ContextMenu::for_desktop(px, py),
                });
                state.pending_full_redraw = true;
                return;
            }

            let now = Instant::now();
            let is_double_click = hit.is_some()
                && state
                    .last_desktop_click
                    .map(|(t, target)| target == hit && now.duration_since(t) < Duration::from_millis(400))
                    .unwrap_or(false);

            state.desktop_icon_selected = hit;

            if is_double_click {
                if let Some(i) = hit {
                    let icon = state.desktop_icons[i].clone();
                    crate::desktop_icons::open_icon(state, &icon);
                }
                // Don't let a third click chain into another open.
                state.last_desktop_click = None;
            } else {
                state.last_desktop_click = Some((now, hit));
            }

            state.pending_full_redraw = true;
            return;
        }
    }

    // ------------------------------------------------------------
    // Super + drag = move / resize.
    // ------------------------------------------------------------
    let logo = state
        .seat
        .get_keyboard()
        .map(|k| k.modifier_state().logo)
        .unwrap_or(false);

    if matches!(button_state, ButtonState::Pressed) && logo {
        if let Some(window) = under.clone() {
            match button {
                BTN_LEFT => crate::wm::begin_move(state, window),
                BTN_RIGHT => crate::wm::begin_resize(state, window),
                _ => {}
            }

            // Swallow Super+drag so clients don't see it.
            return;
        }
    }

    // ------------------------------------------------------------
    // Click-to-focus + raise.
    // ------------------------------------------------------------
    if matches!(button_state, ButtonState::Pressed) {
        if let Some(window) = under.clone() {
            state.space.raise_element(&window, true);
            crate::wm::set_focus(state, Some(window));
        } else {
            // Clicked on empty desktop space; drop focus
            crate::wm::set_focus(state, None);
        }
    }

    // ------------------------------------------------------------
    // Forward button event to the focused surface.
    // ------------------------------------------------------------
    pointer.button(
        state,
        &ButtonEvent {
            button,
            state: button_state,
            serial,
            time: event.time_msec(),
        },
    );
}

/// Handle pointer wheel/axis events.
pub fn handle_pointer_axis<B: InputBackend>(
    state: &mut MitosGuiState,
    event: B::PointerAxisEvent,
) {
    let Some(pointer) = state.seat.get_pointer() else {
        return;
    };

    let mut frame = AxisFrame::new(event.time_msec());

    if let Some(value) = event.amount(Axis::Vertical) {
        frame = frame.value(Axis::Vertical, value);
    }

    if let Some(value) = event.amount(Axis::Horizontal) {
        frame = frame.value(Axis::Horizontal, value);
    }

    // Forward high-resolution (v120) discrete scroll steps for
    // legacy/mouse-wheel clients, when the backend reports them.
    let v120_h = event.amount_v120(Axis::Horizontal);
    let v120_v = event.amount_v120(Axis::Vertical);
    if v120_h.is_some() || v120_v.is_some() {
        frame.v120 = Some((
            v120_h.unwrap_or(0.0) as i32,
            v120_v.unwrap_or(0.0) as i32,
        ));
    }

    pointer.axis(state, frame);
}

/// Find the topmost MITOS window underneath the pointer.
fn window_under_pointer(
    state: &MitosGuiState,
) -> Option<smithay::desktop::Window> {
    state
        .space
        .element_under(state.pointer_location)
        .map(|(window, _)| window.clone())
}

/// Convert the window underneath the pointer into
/// the WlSurface required by Smithay's pointer focus.
/// Returns the surface and the coordinates relative to the surface origin.
fn pointer_focus(state: &MitosGuiState)
    -> Option<(WlSurface, Point<f64, Logical>)>
{
    let (window, location) =
        state.space.element_under(state.pointer_location)?;
    let surface = window.wl_surface()?.into_owned();
    Some((surface, location.to_f64()))
}

/// Check if the pointer is over a dock icon and return its ID.
/// Dispatches one Quick Settings control click. `x` is the pointer's
/// absolute x position and `rect` its control's geometry (as already
/// found by the caller's hit-test) -- needed only for the two sliders,
/// to compute where along the bar it landed; every other control
/// ignores both.
fn handle_quick_settings_click(
    state: &mut MitosGuiState,
    control: crate::quick_settings::QsControl,
    x: i32,
    rect: smithay::utils::Rectangle<i32, smithay::utils::Logical>,
) {
    use crate::quick_settings::{slider_value_at, QsControl};

    match control {
        QsControl::VolumeSlider => {
            state.volume = slider_value_at(rect, x);
            state.muted = false;
            crate::audio::set_volume(state.volume);
            crate::audio::set_muted(false);
        }
        QsControl::MuteToggle => {
            state.muted = !state.muted;
            crate::audio::set_muted(state.muted);
        }
        QsControl::BrightnessSlider => {
            state.brightness = slider_value_at(rect, x);
            crate::brightness::set_brightness(state.brightness);
        }
        QsControl::NightLightToggle => state.toggle_night_light(),
        QsControl::DarkModeToggle => state.toggle_theme_mode(),
        QsControl::DndToggle => state.notifications.dnd = !state.notifications.dnd,
        QsControl::SettingsButton => {
            crate::shell_interaction::launch_app(state, "settings");
            state.quick_settings_visible = false;
        }
    }
}

/// Whether `(px, py)` is outside every shell chrome panel (top bar,
/// dock, the launcher if it's open) -- i.e. it's bare desktop, where
/// desktop icons and the desktop-background context menu live.
fn is_on_bare_desktop(state: &MitosGuiState, px: i32, py: i32) -> bool {
    let in_panel = |panel: Option<crate::renderer::GlassPanel>| {
        panel
            .map(|p| {
                px >= p.position.0
                    && px < p.position.0 + p.size.0
                    && py >= p.position.1
                    && py < p.position.1 + p.size.1
            })
            .unwrap_or(false)
    };

    if in_panel(state.shell.top_bar) || in_panel(state.shell.dock) {
        return false;
    }
    if state.shell.launcher_visible && in_panel(state.shell.launcher) {
        return false;
    }
    true
}

/// Whether a window that's actually *visible right now* covers
/// `(px, py)`: on the active output's current workspace. Not the same
/// question `window_under_pointer` answers -- that asks the Space
/// directly, and windows parked on other workspaces are still mapped
/// in it (just not drawn), so it also "finds" invisible ones. A
/// desktop click needs the visible-only answer: an invisible window
/// over bare desktop shouldn't stop an icon from being clicked.
/// (Hidden windows still being reachable by `window_under_pointer`
/// for click-to-focus and hover is a separate, pre-existing quirk of
/// how workspaces are implemented here, left alone by this change.)
fn visible_window_at(state: &MitosGuiState, px: i32, py: i32) -> bool {
    let output = state.active_output_name();
    let current = state.current_workspace.get(&output).copied().unwrap_or(0);

    state.space.elements().any(|window| {
        let ws = crate::wm::meta(window)
            .workspace
            .get(&output)
            .copied()
            .unwrap_or(0);
        if ws != current {
            return false;
        }
        let Some(location) = state.space.element_location(window) else {
            return false;
        };
        let size = window.geometry().size;
        px >= location.x
            && px < location.x + size.w
            && py >= location.y
            && py < location.y + size.h
    })
}

/// Which desktop icon (if any) is at `(px, py)`, using the same
/// layout `renderer.rs`'s desktop-icon drawing uses (see
/// `desktop_icons::layout_origin_and_rows`).
fn desktop_icon_under_pointer(state: &MitosGuiState, px: i32, py: i32) -> Option<usize> {
    let (top_bar_h, max_rows) = crate::desktop_icons::layout_origin_and_rows(state);
    let (cell_w, cell_h) = crate::desktop_icons::cell_size();

    for i in 0..state.desktop_icons.len() {
        let (cx, cy) = crate::desktop_icons::icon_position(i, max_rows);
        let (x, y) = (cx, top_bar_h + cy);
        if px >= x && px < x + cell_w && py >= y && py < y + cell_h {
            return Some(i);
        }
    }
    None
}

fn dock_icon_under_pointer(
    state: &MitosGuiState,
) -> Option<&'static str> {
    let dock = state.shell.dock.as_ref()?;
    let layout = &state.shell.dock_layout;

    if layout.items.is_empty() {
        return None;
    }

    let pointer = state.pointer_location;
    let icon_size = layout.icon_size as f64;
    let spacing = layout.spacing as f64;

    let total_width = (layout.items.len() as f64 * icon_size)
        + ((layout.items.len().saturating_sub(1)) as f64 * spacing);

    let start_x = dock.position.0 as f64
        + ((dock.size.0 as f64 - total_width) / 2.0).max(0.0);

    let baseline = (dock.position.1 + dock.size.1 - 8) as f64;

    for (index, item) in layout.items.iter().enumerate() {
        let x = start_x + index as f64 * (icon_size + spacing);
        let y = baseline - icon_size;

        if pointer.x >= x
            && pointer.x <= x + icon_size
            && pointer.y >= y
            && pointer.y <= y + icon_size
        {
            return Some(item.id);
        }
    }

    None
}

/// Check if the pointer is in a hot corner and trigger the associated action.
pub fn check_hot_corners(
    state: &mut MitosGuiState, 
    pointer: Point<f64, Logical>, 
    output_size: Size<i32, Logical>
) {
    // 500ms cooldown prevents rapid-fire toggling when the mouse rests in the corner
    if state.hot_corners_last_triggered.elapsed() < Duration::from_millis(500) {
        return;
    }

    let (x, y) = (pointer.x, pointer.y);
    let (w, _h) = (output_size.w as f64, output_size.h as f64);
    let threshold = 5.0; // 5 logical pixels from the edge

    // Top-Left: Open Launcher
    if x < threshold && y < threshold && !state.shell.launcher_visible {
        crate::keyboard::toggle_launcher(state);
        state.hot_corners_last_triggered = Instant::now();
    }
    
    // Top-Right: Toggle Night Light
    if x > w - threshold && y < threshold {
        state.toggle_night_light();
        state.hot_corners_last_triggered = Instant::now();
    }
}
