//! Keyboard input.
//!
//! Stage 3 keyboard handling:
//! - forward normal keys to the focused Wayland client
//! - Super + Space toggles the MITOS launcher

use smithay::backend::input::{
    Event,
    InputBackend,
    KeyboardKeyEvent,
};

use smithay::input::keyboard::{
    keysyms,
    FilterResult,
};

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
        |state, mods, sym| {
            let keysym = sym.modified_sym();

            // --------------------------------------------------------
            // MITOS launcher shortcut
            //
            // Super + Space
            // --------------------------------------------------------

            if mods.logo && keysym == keysyms::KEY_space.into() {
                state.shell.toggle_launcher();

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
            // Normal keyboard input
            // --------------------------------------------------------

            FilterResult::Forward
        },
    );
}

/// Toggle the MITOS launcher programmatically.
pub fn toggle_launcher(state: &mut MitosGuiState) {
    state.shell.toggle_launcher();
}
