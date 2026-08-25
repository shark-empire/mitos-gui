//! Keyboard input.
//!
//! Stage 3 keyboard handling:
//! - forward normal keys to the focused Wayland client
//! - launcher shortcut handling will be added through the
//!   Smithay 0.7 keysym API.

use smithay::backend::input::{
    Event,
    InputBackend,
    KeyboardKeyEvent,
};

use smithay::input::keyboard::FilterResult;

use smithay::utils::SERIAL_COUNTER;

use crate::state::MitosGuiState;

/// Feed one raw keyboard event into the MITOS seat.
pub fn handle_keyboard_key<B: InputBackend>(
    state: &mut MitosGuiState,
    event: B::KeyboardKeyEvent,
) {
    let Some(keyboard) = state.seat.get_keyboard() else {
        return;
    };

    let serial = SERIAL_COUNTER.next_serial();
    let time = event.time_msec();
    let keycode = event.key_code();
    let key_state = event.state();

    keyboard.input::<(), _>(
        state,
        keycode,
        key_state,
        serial,
        time,
        |_state, _mods, _sym| {
            FilterResult::Forward
        },
    );
}

/// Toggle the MITOS launcher.
pub fn toggle_launcher(
    state: &mut MitosGuiState,
) {
    state.shell.launcher_visible =
        !state.shell.launcher_visible;
}
