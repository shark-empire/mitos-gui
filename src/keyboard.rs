//! Keyboard input.
//!
//! MITOS intercepts compositor-level shortcuts here and forwards
//! everything else to the focused Wayland client.

use smithay::backend::input::{
    Event,
    InputBackend,
    KeyState,
    KeyboardKeyEvent,
};

use smithay::input::keyboard::{
    keysyms,
    FilterResult,
};

use smithay::utils::SERIAL_COUNTER;

use crate::state::MitosGuiState;

/// Feeds one raw key event into the seat's keyboard handle.
///
/// Current MITOS shortcuts:
///
/// - Super + Space -> toggle launcher
///
/// Everything else is forwarded to the focused client.
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
    |state, mods, sym| {
        if mods.logo && sym == keysyms::KEY_space.into() {
            state.shell.launcher_visible =
                !state.shell.launcher_visible;

            return FilterResult::Intercept(());
        }

        FilterResult::Forward
    },
);
}

/// Toggle the MITOS application launcher.
pub fn toggle_launcher(
    state: &mut MitosGuiState,
) {
    state.shell.toggle_launcher();
}
