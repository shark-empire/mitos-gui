/// Feeds one raw key event into the seat's keyboard handle.
///
/// MITOS intercepts compositor-level shortcuts before forwarding
/// the key to the focused Wayland client.
///
/// Current shortcuts:
///
/// - Super + Space → toggle launcher
///
/// Stage 4 will expand this into:
///
/// - Super + Q       → close window
/// - Super + W       → move window
/// - Super + arrows  → resize/move
/// - Super + 1..9    → workspace switching
/// - Super + M       → maximize
///
/// All other keys are forwarded normally to the focused client.

use smithay::backend::input::{
    Event,
    InputBackend,
    KeyboardKeyEvent,
    KeyState,
};
use smithay::input::keyboard::FilterResult;
use smithay::utils::SERIAL_COUNTER;

use crate::state::MitosGuiState;
use smithay::input::keyboard::Keysym;


pub fn handle_keyboard_key<B: InputBackend>(state: &mut MitosGuiState, event: B::KeyboardKeyEvent) {
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
    |state, mods, keysym| {
        if key_state == KeyState::Pressed {
            if mods.logo && keysym == Keysym::space {
                toggle_launcher(state);

                return FilterResult::Intercept;
            }
        }

        FilterResult::Forward
    },
  );
}



pub fn toggle_launcher(state: &mut MitosGuiState) {
    state.shell.toggle_launcher();
}
