//! Keyboard input.
//!
//! Stage 4 keyboard handling:
//! - forward normal keys to the focused Wayland client
//! - Super + Space toggles the MITOS launcher
//! - Stage 4 window manager shortcuts (close, maximize, snap, etc.)

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
            // Stage 4 Window Manager shortcuts
            // --------------------------------------------------------

            if mods.logo {
                // Super + Q: Close focused window
                if keysym == keysyms::KEY_q.into() {
                    crate::wm::close_focused(state);
                    return FilterResult::Intercept(());
                }

                // Super + F: Toggle fullscreen
                if keysym == keysyms::KEY_f.into() {
                    crate::wm::toggle_fullscreen(state);
                    return FilterResult::Intercept(());
                }

                // Super + Up: Toggle maximize
                if keysym == keysyms::KEY_Up.into() {
                    crate::wm::toggle_maximize(state);
                    return FilterResult::Intercept(());
                }

                // Super + Down: Minimize
                // Super + Shift + Down: Restore last minimized
                if keysym == keysyms::KEY_Down.into() {
                    if mods.shift {
                        crate::wm::restore_minimized(state);
                    } else {
                        crate::wm::minimize_focused(state);
                    }
                    return FilterResult::Intercept(());
                }

                // Super + Left: Snap to left half
                if keysym == keysyms::KEY_Left.into() {
                    crate::wm::snap(state, crate::wm::SnapSide::Left);
                    return FilterResult::Intercept(());
                }

                // Super + Right: Snap to right half
                if keysym == keysyms::KEY_Right.into() {
                    crate::wm::snap(state, crate::wm::SnapSide::Right);
                    return FilterResult::Intercept(());
                }

                // Super + Tab: Cycle focus through mapped windows
                if keysym == keysyms::KEY_Tab.into() {
                    crate::wm::cycle_focus(state);
                    return FilterResult::Intercept(());
                }
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
