//! MITOS pointer input.
//!
//! Stage 3:
//! - track the compositor pointer position
//! - find the window underneath the pointer
//! - focus the window on button press
//! - forward pointer events to the focused Wayland surface
//!
//! Stage 4 will add:
//! - window dragging
//! - resizing
//! - decorations
//! - launcher interaction
//! - dock interaction

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
    SERIAL_COUNTER,
};

use smithay::wayland::seat::WaylandFocus;

use crate::state::MitosGuiState;

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

    state.pointer_location =
        Point::<f64, Logical>::from((x, y));

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

    let button_state = match event.state() {
        ButtonState::Pressed => ButtonState::Pressed,
        ButtonState::Released => ButtonState::Released,
    };

    // Find the window underneath the pointer.
    let under = window_under_pointer(state);

    // ------------------------------------------------------------
    // Focus the clicked window.
    // ------------------------------------------------------------

    if matches!(button_state, ButtonState::Pressed) {
        if let Some(window) = under {
            state.space.raise_element(&window, true);
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

    let mut frame =
        AxisFrame::new(event.time_msec());

    if let Some(value) =
        event.amount(Axis::Vertical)
    {
        frame =
            frame.value(
                Axis::Vertical,
                value,
            );
    }

    if let Some(value) =
        event.amount(Axis::Horizontal)
    {
        frame =
            frame.value(
                Axis::Horizontal,
                value,
            );
    }

    pointer.axis(
        state,
        frame,
    );
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
fn pointer_focus(
    state: &MitosGuiState,
) -> Option<(
    smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    Point<f64, Logical>,
)> {
    let window = window_under_pointer(state)?;

    let surface = window
        .wl_surface()?
        .into_owned();

    Some((
        surface,
        state.pointer_location,
    ))
}
