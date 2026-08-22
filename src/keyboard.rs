//! Keyboard input.
//!
//! Winit is both our display backend and, for now, our input source --
//! it forwards host keyboard events into the same shape a real
//! libinput backend will produce once Stage 5 lands, so nothing here
//! is winit-specific beyond the generic `InputBackend` parameter.

use smithay::backend::input::{Event, InputBackend, KeyboardKeyEvent};
use smithay::input::keyboard::FilterResult;
use smithay::utils::SERIAL_COUNTER;

use crate::state::MitosGuiState;

/// Feeds one raw key event into the seat's keyboard handle.
///
/// Smithay tracks keymap/modifier state internally and forwards the
/// result to whichever surface currently has keyboard focus. There's
/// no compositor-level keybinding table yet -- every key just passes
/// straight through to the client. That's where Stage 4's shortcuts
/// (raise/close/switch-workspace, etc.) hook in: they'd intercept
/// specific keysyms here and return `FilterResult::Intercept` instead
/// of `Forward`.
pub fn handle_keyboard_key<B: InputBackend>(state: &mut MitosGuiState, event: B::KeyboardKeyEvent) {
    let Some(keyboard) = state.seat.get_keyboard() else {
        return;
    };

    let serial = SERIAL_COUNTER.next_serial();
    let time = event.time_msec();
    let keycode = event.key_code();
    let key_state = event.state();

    keyboard.input::<(), _>(state, keycode, key_state, serial, time, |_state, _mods, _keysym| {
        FilterResult::Forward
    });
}
