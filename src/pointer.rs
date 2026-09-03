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
    // Dock icon click.
    // ------------------------------------------------------------
    if matches!(button_state, ButtonState::Pressed) && button == BTN_LEFT {
        if let Some(app_id) = dock_icon_under_pointer(state) {
            crate::shell_interaction::launch_app(state, app_id);
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
        state.shell.toggle_launcher();
        state.pending_full_redraw = true;
        state.hot_corners_last_triggered = Instant::now();
    }
    
    // Top-Right: Toggle Night Light
    if x > w - threshold && y < threshold {
        state.toggle_night_light();
        state.hot_corners_last_triggered = Instant::now();
    }
}
