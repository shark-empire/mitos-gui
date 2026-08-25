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
    ButtonState,
    Event,
    InputBackend,
    PointerAxisEvent,
    PointerButtonEvent,
    PointerMotionEvent,
};

use smithay::input::pointer::{
    AxisFrame,
    ButtonEvent,
    MotionEvent,
};

use smithay::utils::{
    Logical,
    Point,
    SERIAL_COUNTER,
};

use crate::state::MitosGuiState;

/// Handle pointer motion.
pub fn handle_pointer_motion<B: InputBackend>(
    state: &mut MitosGuiState,
    event: B::PointerMotionEvent,
) {
    let delta = event.delta();

    let new_location = Point::<f64, Logical> {
        x: state.pointer_location.x + delta.0,
        y: state.pointer_location.y + delta.1,
    };

    state.pointer_location = new_location;

    let Some(pointer) = state.seat.get_pointer() else {
        return;
    };

    let serial = SERIAL_COUNTER.next_serial();

    let time = event.time_msec();

    let under = window_under_pointer(state);

    pointer.motion(
        state,
        under,
        &MotionEvent {
            location: state.pointer_location,
            serial,
            time,
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

    let state_button = match event.state() {
        ButtonState::Pressed => ButtonState::Pressed,
        ButtonState::Released => ButtonState::Released,
    };

    let under = window_under_pointer(state);

    // ------------------------------------------------------------
    // Focus clicked window.
    // ------------------------------------------------------------

    if matches!(state_button, ButtonState::Pressed) {
        if let Some(window) = under.clone() {
            state.space.raise_element(&window, true);
        }
    }

    pointer.button(
        state,
        &ButtonEvent {
            button,
            state: state_button,
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

    let amount = event.amount(Axis::Vertical);

    if let Some(amount) = amount {
        frame = frame.value(Axis::Vertical, amount);
    }

    pointer.axis(state, frame);
}

/// Find the topmost MITOS window beneath the pointer.
fn window_under_pointer(
    state: &MitosGuiState,
) -> Option<smithay::desktop::Window> {
    state
        .space
        .element_under(
            state.pointer_location,
        )
        .map(|(window, _location)| window.clone())
}
