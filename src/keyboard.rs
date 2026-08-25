//! Keyboard input.
//!
//! Stage 3 keyboard handling:
//! - forward normal keys to the focused Wayland client
//! - intercept Super + Space
//! - toggle the MITOS launcher

use smithay::backend::input::{
    Event,
    InputBackend,
    KeyboardKeyEvent,
};

use smithay::input::keyboard::{
    FilterResult,
    KeysymHandle,
};

use smithay::utils::SERIAL_COUNTER;

use xkbcommon::xkb;

use crate::state::MitosGuiState;

/// Feeds one raw key event into the seat keyboard.
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
            // --------------------------------------------------------
            // MITOS launcher shortcut
            //
            // Super + Space
            // --------------------------------------------------------

            if mods.logo
                && sym.modified_sym()
                    == xkb::KEY_space
            {
                state.shell.launcher_visible =
                    !state.shell.launcher_visible;

                tracing::info!(
                    "MITOS: launcher {}",
                    if state.shell.launcher_visible {
                        "opened"
                    } else {
                        "closed"
                    }
                );

                return FilterResult::Intercept(());
            }

            // --------------------------------------------------------
            // Everything else goes to the focused client.
            // --------------------------------------------------------

            FilterResult::Forward
        },
    );
}

/// Toggle the MITOS launcher programmatically.
pub fn toggle_launcher(
    state: &mut MitosGuiState,
) {
    state.shell.launcher_visible =
        !state.shell.launcher_visible;
}
