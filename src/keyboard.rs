//! Keyboard input.
//!
//! Stage 4 & 5 keyboard handling:
//! - Launcher search navigation (when open)
//! - Super + Space toggles the MITOS launcher
//! - Stage 4 window manager shortcuts (close, maximize, snap, etc.)
//! - Forward normal keys to the focused Wayland client

use smithay::backend::input::{
    Event,
    InputBackend,
    KeyboardKeyEvent,
    KeyState,
};

use smithay::input::keyboard::{
    keysyms,
    keysym_to_char,
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
            // LAUNCHER SEARCH NAVIGATION
            // If the launcher is open, it captures ALL keyboard input.
            // --------------------------------------------------------
            if state.shell.launcher_visible {
                return handle_launcher_input(state, keysym, key_state);
            }

            // --------------------------------------------------------
            // MITOS launcher shortcut
            //
            // Super + Space
            // --------------------------------------------------------
            if mods.logo && keysym == keysyms::KEY_space.into() {
                state.shell.toggle_launcher();
                state.pending_full_redraw = true; // Force redraw to show/hide launcher

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

/// Handles typing, navigation, and execution inside the open launcher.
fn handle_launcher_input(
    state: &mut MitosGuiState,
    keysym: u32,
    key_state: KeyState,
) -> FilterResult<()> {
    // Only act on key presses, ignore releases
    if key_state != KeyState::Pressed {
        return FilterResult::Intercept(());
    }

    match keysym {
        keysyms::KEY_Escape => {
            state.shell.toggle_launcher();
        }
        keysyms::KEY_Return => {
            // Launch the currently selected app
            if let Some(app) = state.shell.launcher_results.get(state.shell.launcher_selected) {
                crate::shell_interaction::launch_app_entry(app);
            }
            state.shell.toggle_launcher();
        }
        keysyms::KEY_BackSpace => {
            state.shell.launcher_query.pop();
            update_launcher_results(state);
        }
        keysyms::KEY_Down => {
            if !state.shell.launcher_results.is_empty() {
                state.shell.launcher_selected = 
                    (state.shell.launcher_selected + 1) % state.shell.launcher_results.len();
            }
        }
        keysyms::KEY_Up => {
            if !state.shell.launcher_results.is_empty() {
                if state.shell.launcher_selected == 0 {
                    state.shell.launcher_selected = state.shell.launcher_results.len() - 1;
                } else {
                    state.shell.launcher_selected -= 1;
                }
            }
        }
        _ => {
            // Convert keysym to a character and append to query
            if let Some(c) = keysym_to_char(keysym) {
                if c.is_ascii_graphic() || c == ' ' {
                    state.shell.launcher_query.push(c);
                    update_launcher_results(state);
                }
            }
        }
    }

    state.pending_full_redraw = true; // Force a redraw to show the new text/selection
    FilterResult::Intercept(())
}

/// Re-runs the search algorithm when the query changes.
fn update_launcher_results(state: &mut MitosGuiState) {
    state.shell.launcher_results = crate::shell_interaction::search_apps(
        &state.launcher_apps,
        &state.shell.launcher_query,
    );
    state.shell.launcher_selected = 0; // Reset selection to top result
}

/// Toggle the MITOS launcher programmatically.
pub fn toggle_launcher(state: &mut MitosGuiState) {
    state.shell.toggle_launcher();
    state.pending_full_redraw = true;
}
