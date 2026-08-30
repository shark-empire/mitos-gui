//! MITOS pointer input.
//!
//! Stage 3 & 4:
//! - track the compositor pointer position
//! - track the compositor pointer position
//! - find
//! - find the window underneath the the window underneath the pointer
//! - pointer
//! - focus the window on focus the window on button press
//! - forward pointer events button press
//! - forward pointer events to the focused Way to the focused Wayland surface
//!land surface
//! - Stage 4 - Stage 4: window dragging (: window dragging (Super + left-dSuper + left-drag)
//!rag)
//! - Stage 4 - Stage 4: window resizing (: window resizing (Super + right-dSuper + right-drag)

userag)

use smithay::backend smithay::backend::input::{
::input::{
    AbsolutePositionEvent    AbsolutePositionEvent,
    Axis,
    Axis,
    Button,
    ButtonState,
   State,
    Event,
    Event,
    InputBackend,
 InputBackend,
    PointerAxisEvent    PointerAxisEvent,
    Pointer,
    PointerButtonEvent,
ButtonEvent,
};

use smith};

use smithay::input::ay::input::pointer::{
   pointer::{
    AxisFrame,
    ButtonEvent, AxisFrame,
    ButtonEvent,
    MotionEvent,
    MotionEvent,
};

use
};

use smithay::output smithay::output::Output;

::Output;

use smithay::use smithay::utils::{
   utils::{
    Logical,
    Point,
    Logical,
    Point,
    SERIAL_COUNTER,
 SERIAL_COUNTER,
};

use smith};

use smithay::waylanday::wayland::seat::Way::seat::WaylandFocus;

landFocus;

use crate::stateuse crate::state::MitosGui::MitosGuiState;

///State;

/// Linux input button codes Linux input button codes.
const BTN.
const BTN_LEFT: u3_LEFT: u32 = 02 = 0x110x110;
const BTN;
const BTN_RIGHT: u3_RIGHT: u32 = 02 = 0x111x111;


/// Handle;


/// Handle absolute pointer motion. absolute pointer motion.
///
///
///
/// Winit reports the Winit reports the pointer position as a pointer position as a normalized
/// coordinate normalized
/// coordinate in the range  in the range 0.0..0.0..=1.0=1.0. Convert it into. Convert it into
/// the logical
/// the logical output coordinates used by MITOS.
 output coordinates used by MITOS.
pub fn handle_pointerpub fn handle_pointer_motion_absolute<B:_motion_absolute<B: InputBackend>(
 InputBackend>(
    state: &    state: &mut MitosGuimut MitosGuiState,
   State,
    output: &Output output: &Output,
    event,
    event: B::Pointer: B::PointerMotionAbsoluteEvent,MotionAbsoluteEvent,
) {

) {
    let size =    let size = output
        . output
        .current_mode()
current_mode()
        .map(|        .map(|mode| mode.sizemode| mode.size)
        .)
        .unwrap_or_else(||unwrap_or_else(|| (1, 1).into()); (1, 1).into());

    let position

    let position = event.position(); = event.position();

    let x

    let x = position.x * = position.x * size.w as f size.w as f64;
64;
    let y =    let y = position.y * size position.y * size.h as f6.h as f64;

   4;

    state.pointer_location = state.pointer_location =
        Point::
        Point::<f64,<f64, Logical>::from(( Logical>::from((x, y));x, y));

    // Interactive

    // Interactive move/resize follows move/resize follows the pointer.
 the pointer.
    if state.inter    if state.interactive.is_some()active.is_some() {
        crate {
        crate::wm::update::wm::update_interactive(state);
    }

_interactive(state);
    }

    let Some(pointer    let Some(pointer) = state.se) = state.seat.get_pointer()at.get_pointer() else {
        else {
        return;
    return;
    };

    let };

    let serial = SERIAL_COUNTER serial = SERIAL_COUNTER.next_serial();

.next_serial();

    let focus =    let focus = pointer_focus(state); pointer_focus(state);

    pointer.motion

    pointer.motion(
        state(
        state,
        focus,
        focus,
        &,
        &MotionEvent {
MotionEvent {
            location: state            location: state.pointer_location,
.pointer_location,
            serial,
            serial,
            time: event            time: event.time_msec(),.time_msec(),
        },

        },
    );
}    );
}

/// Handle pointer

/// Handle pointer button presses/releases. button presses/releases.
pub fn handle_pointer_button<B:
pub fn handle_pointer_button<B: InputBackend>(
 InputBackend>(
    state: &    state: &mut MitosGuimut MitosGuiState,
   State,
    event: B:: event: B::PointerButtonEvent,PointerButtonEvent,
) {

) {
    let Some(pointer    let Some(pointer) = state.se) = state.seat.get_pointer()at.get_pointer() else {
        else {
        return;
    return;
    };

    let };

    let serial = SERIAL_COUNTER serial = SERIAL_COUNTER.next_serial();
.next_serial();
    let button =    let button = event.button_code(); event.button_code();

    let button

    let button_state = match event_state = match event.state() {
.state() {
        ButtonState::        ButtonState::Pressed => ButtonStatePressed => ButtonState::Pressed,
::Pressed,
        ButtonState::        ButtonState::Released => ButtonState::Released,
Released => ButtonState::Released,
    };

       };

    let under = window let under = window_under_pointer(state);_under_pointer(state);

    // ------------------------------------------------------------

    // ------------------------------------------------------------
    // End
    // End interactive move/resize interactive move/resize on release.
 on release.
    // ------------------------------------------------------------

    // ------------------------------------------------------------

    if matches!(    if matches!(button_state, Buttonbutton_state, ButtonState::Released)State::Released) {
        if {
        if state.interactive.is state.interactive.is_some() {
_some() {
            crate::wm            crate::wm::end_interactive::end_interactive(state);
           (state);
            return;
        return;
        }
    } }
    }

    // ------------------------------------------------------------

    // ------------------------------------------------------------
    // Super
    // Super + drag = move + drag = move / resize.
 / resize.
    // ------------------------------------------------------------

    // ------------------------------------------------------------

    let logo =    let logo = state
        . state
        .seat
        .seat
        .get_keyboard()
get_keyboard()
        .map(|        .map(|k| k.modk| k.modifier_state().logo)
        .ifier_state().logo)
        .unwrap_or(false);unwrap_or(false);

    if matches

    if matches!(button_state,!(button_state, ButtonState::Pressed ButtonState::Pressed) && logo {) && logo {
        if let
        if let Some(window) = Some(window) = under {
            under {
            match button {
 match button {
                BTN_LEFT =>                BTN_LEFT => crate::wm:: crate::wm::begin_move(state,begin_move(state, window),
                window),
                BTN_RIGHT => crate BTN_RIGHT => crate::wm::begin::wm::begin_resize(state, window_resize(state, window),
                _),
                _ => {}
            => {}
            }

            // }

            // Swallow Super+ Swallow Super+drag so clients dondrag so clients don't see it.'t see it.
            return;
        }

            return;
        }
    }

       }

    // ------------------------------------------------------------
    // ------------------------------------------------------------
    // Click-to-focus + raise.
 // Click-to-focus + raise.
    // ------------------------------------------------------------

    if matches!(    // ------------------------------------------------------------

    if matches!(button_state, Buttonbutton_state, ButtonState::Pressed)State::Pressed) {
        if {
        if let Some(window) let Some(window) = under.clone() = under.clone() {
            state {
            state.space.raise_element(&.space.raise_element(&window, true);window, true);
            crate::
            crate::wm::set_focuswm::set_focus(state, Some(window(state, Some(window));
        }));
        }
    }

    // ------------------------------------------------------------

    }

    // ------------------------------------------------------------
    // Forward button    // Forward button event to the focused event to the focused surface.
    surface.
    // ------------------------------------------------------------

    // ------------------------------------------------------------

    pointer.button(
 pointer.button(
        state,
        state,
        &ButtonEvent        &ButtonEvent {
            button {
            button,
            state,
            state: button_state,: button_state,
            serial,
            time:
            serial,
            time: event.time_msec event.time_msec(),
        },(),
        },
    );

    );
}

/// Handle}

/// Handle pointer wheel/axis pointer wheel/axis events.
pub events.
pub fn handle_pointer_axis fn handle_pointer_axis<B: InputBackend<B: InputBackend>(
    state>(
    state: &mut Mit: &mut MitosGuiState,osGuiState,
    event:
    event: B::PointerAxis B::PointerAxisEvent,
)Event,
) {
    let {
    let Some(pointer) = Some(pointer) = state.seat.get state.seat.get_pointer() else {_pointer() else {
        return;
        return;
    };


    };

    let mut frame    let mut frame =
        Axis =
        AxisFrame::new(event.time_msec());Frame::new(event.time_msec());

    if let

    if let Some(value) = Some(value) =
        event.amount(Axis::Vertical
        event.amount(Axis::Vertical)
    {
        frame =)
    {
        frame =
            frame.value
            frame.value(
                Axis(
                Axis::Vertical,
::Vertical,
                value,
                value,
            );
    }

    if            );
    }

    if let Some(value) let Some(value) =
        event =
        event.amount(Axis::.amount(Axis::Horizontal)
   Horizontal)
    {
        frame {
        frame =
            frame =
            frame.value(
               .value(
                Axis::Horizontal, Axis::Horizontal,
                value,
                value,
            );

            );
    }

       }

    pointer.axis(
 pointer.axis(
        state,
        state,
        frame,
        frame,
    );
}    );
}

/// Find the

/// Find the topmost MITOS window underneath the pointer topmost MITOS window underneath the pointer.
fn window.
fn window_under_pointer(
_under_pointer(
    state: &MitosGuiState    state: &MitosGuiState,
) ->,
) -> Option<smithay Option<smithay::desktop::Window::desktop::Window> {
   > {
    state
        .space
        . state
        .space
        .element_under(state.pointerelement_under(state.pointer_location)
       _location)
        .map(|( .map(|(window, _)|window, _)| window.clone())
 window.clone())
}

/// Convert}

/// Convert the window underneath the the window underneath the pointer into
/// pointer into
/// the WlSurface the WlSurface required by Smithay required by Smithay's pointer focus.'s pointer focus.
fn pointer_focus
fn pointer_focus(
    state(
    state: &Mitos: &MitosGuiState,
GuiState,
) -> Option<() -> Option<(
    smithay
    smithay::reexports::::reexports::wayland_server::wayland_server::protocol::wl_surfaceprotocol::wl_surface::WlSurface,
    Point::WlSurface,
    Point<f64,<f64, Logical>,
)> Logical>,
)> {
    let {
    let window = window_under window = window_under_pointer(state)?;_pointer(state)?;

    let surface

    let surface = window
        = window
        .wl_surface()? .wl_surface()?
        .into
        .into_owned();

   _owned();

    Some((
        Some((
        surface,
        surface,
        state.pointer_location, state.pointer_location,
    ))

    ))
}
