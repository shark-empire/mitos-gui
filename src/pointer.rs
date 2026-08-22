//! Pointer input: cursor motion, clicks, and scroll -- plus the simple
//! click-to-focus policy standing in for Stage 4's real window manager.

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, ButtonState, Event, InputBackend, PointerAxisEvent,
    PointerButtonEvent,
};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::output::Output;
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::seat::WaylandFocus;

use crate::state::MitosGuiState;

/// Handles absolute pointer motion -- the only kind of motion event
/// winit's virtual mouse produces (it always reports a position within
/// the window, never a relative delta).
pub fn handle_pointer_motion_absolute<B: InputBackend>(
    state: &mut MitosGuiState,
    output: &Output,
    event: B::PointerMotionAbsoluteEvent,
) {
    let output_size = output
        .current_mode()
        .map(|mode| mode.size)
        .unwrap_or_else(|| (0, 0).into());

    let location = (
        event.x_transformed(output_size.w),
        event.y_transformed(output_size.h),
    )
        .into();

    state.pointer_location = location;

    // Resolve who's under the new position *before* touching `state`
    // mutably again below -- `element_under` borrows `state.space`,
    // and that borrow needs to be fully resolved into owned values
    // (not references into the Space) before `pointer.motion(state, ..)`
    // needs `state` back.
    let focus = state
        .space
        .element_under(location)
        .and_then(|(window, window_loc)| {
            window
                .wl_surface()
                .map(|surface| (surface.into_owned(), window_loc.to_f64()))
        });

    let serial = SERIAL_COUNTER.next_serial();
    let time = event.time_msec();

    if let Some(pointer) = state.seat.get_pointer() {
        pointer.motion(
            state,
            focus,
            &MotionEvent {
                location,
                serial,
                time,
            },
        );
        pointer.frame(state);
    }
}

/// Handles a pointer button press/release.
///
/// On press, this also implements click-to-focus: whatever's under the
/// cursor gets raised to the top of the stack and takes keyboard focus.
/// It's a stand-in for Stage 4's real window manager, which will also
/// need to handle click-to-move/resize via the same surface-under-point
/// lookup.
pub fn handle_pointer_button<B: InputBackend>(state: &mut MitosGuiState, event: B::PointerButtonEvent) {
    let serial = SERIAL_COUNTER.next_serial();
    let time = event.time_msec();
    let button = event.button_code();
    let button_state = event.state();

    if button_state == ButtonState::Pressed {
        // Same reasoning as in the motion handler: fully resolve into
        // an owned `Window` (cheap -- it's a ref-counted handle) before
        // borrowing `state` mutably again for the raise/focus calls.
        let clicked = state
            .space
            .element_under(state.pointer_location)
            .map(|(window, _)| window.clone());

        if let Some(window) = clicked {
            state.space.raise_element(&window, true);

            if let Some(surface) = window.wl_surface() {
                if let Some(keyboard) = state.seat.get_keyboard() {
                    keyboard.set_focus(state, Some(surface.into_owned()), serial);
                }
            }
        }
    }

    if let Some(pointer) = state.seat.get_pointer() {
        pointer.button(
            state,
            &ButtonEvent {
                button,
                state: button_state,
                serial,
                time,
            },
        );
        pointer.frame(state);
    }
}

/// Handles a scroll-wheel / trackpad axis event.
pub fn handle_pointer_axis<B: InputBackend>(state: &mut MitosGuiState, event: B::PointerAxisEvent) {
    let time = event.time_msec();
    let source = event.source();

    let mut frame = AxisFrame::new(time).source(source);

    if let Some(horizontal) = event.amount(Axis::Horizontal) {
        frame = frame.value(Axis::Horizontal, horizontal);
    }

    if let Some(vertical) = event.amount(Axis::Vertical) {
        frame = frame.value(Axis::Vertical, vertical);
    }

    if let Some(pointer) = state.seat.get_pointer() {
        pointer.axis(state, frame);
        pointer.frame(state);
    }
}
