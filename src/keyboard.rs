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
            // SECURE AUTHENTICATION PROMPT (Highest Priority)
            // --------------------------------------------------------
            if state.auth.active {
                return handle_auth_input(state, keysym, key_state);
            }


            // --------------------------------------------------------
            // Stage 6: hardware media keys
            // --------------------------------------------------------
            if key_state == KeyState::Pressed {
                match keysym {
                    keysyms::KEY_XF86AudioRaiseVolume => {
                        state.muted = false;
                        state.volume = state.volume.saturating_add(5).min(100);
                        state.notifications.push(
                            "MITOS Audio",
                            &format!("Volume: {}%", state.volume),
                            "Applied by the audio service in Stage 8.",
                        );
                        state.pending_full_redraw = true;
                        return FilterResult::Intercept(());
                    }

                    keysyms::KEY_XF86AudioLowerVolume => {
                        state.muted = false;
                        state.volume = state.volume.saturating_sub(5);
                        state.notifications.push(
                            "MITOS Audio",
                            &format!("Volume: {}%", state.volume),
                            "Applied by the audio service in Stage 8.",
                        );
                        state.pending_full_redraw = true;
                        return FilterResult::Intercept(());
                    }

                    keysyms::KEY_XF86AudioMute => {
                        state.muted = !state.muted;
                        state.notifications.push(
                            "MITOS Audio",
                            if state.muted { "Muted" } else { "Unmuted" },
                            "",
                        );
                        state.pending_full_redraw = true;
                        return FilterResult::Intercept(());
                    }

                    _ => {}
                }
            }


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

             // Super + N: Push a test notification (Stage 6)
                if keysym == keysyms::KEY_n.into() {
                    state.notifications.push(
                        "MITOS System",
                        "Notification Engine Active",
                        "Stage 6 desktop services are online.",
                    );
                    state.pending_full_redraw = true;
                    return FilterResult::Intercept(());
                }

                // Super + Shift + A: Trigger mock auth prompt
                if mods.shift && keysym == keysyms::KEY_a.into() {
                    state.auth.request("MITOS Package Manager", "Install system updates");
                    state.pending_full_redraw = true;
                    return FilterResult::Intercept(());
                }

                // Super + Shift + R: Reboot via mitos-init
                if mods.shift && keysym == keysyms::KEY_r.into() {
                    crate::session::reboot();
                    return FilterResult::Intercept(());
                }
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

                // Super + 1/2/3/4: Switch Workspace
                if keysym == keysyms::KEY_1.into() { state.switch_workspace(0); return FilterResult::Intercept(()); }
                if keysym == keysyms::KEY_2.into() { state.switch_workspace(1); return FilterResult::Intercept(()); }
                if keysym == keysyms::KEY_3.into() { state.switch_workspace(2); return FilterResult::Intercept(()); }
                if keysym == keysyms::KEY_4.into() { state.switch_workspace(3); return FilterResult::Intercept(()); }

                // Super + Shift + 1/2/3/4: Move focused window to Workspace
                if mods.shift {
                    if let Some(win) = state.focused_window.clone() {
                        let target = match keysym {
                            keysyms::KEY_1 => Some(0),
                            keysyms::KEY_2 => Some(1),
                            keysyms::KEY_3 => Some(2),
                            keysyms::KEY_4 => Some(3),
                            _ => None,
                        };
                        if let Some(t) = target {
                            crate::wm::meta(&win).workspace = t;
                            state.switch_workspace(t); // Follow the window
                            return FilterResult::Intercept(());
                        }
                    }
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

fn handle_auth_input(
    state: &mut MitosGuiState,
    keysym: u32,
    key_state: KeyState,
) -> FilterResult<()> {
    if key_state != KeyState::Pressed {
        return FilterResult::Intercept(());
    }

    match keysym {
        keysyms::KEY_Escape => {
            state.auth.cancel();
            state.notifications.push("MITOS Security", "Authentication cancelled", "");
        }
        keysyms::KEY_Return => {
            if state.auth.submit() {
                state.notifications.push("MITOS Security", "Authentication successful", "Privileges granted.");
            } else {
                state.notifications.push("MITOS Security", "Authentication failed", "Incorrect password.");
            }
        }
        keysyms::KEY_BackSpace => {
            state.auth.password.pop();
        }
        _ => {
            if let Some(c) = keysym_to_char(keysym) {
                if c.is_ascii_graphic() || c == ' ' {
                    state.auth.password.push(c);
                }
            }
        }
    }

    state.pending_full_redraw = true;
    FilterResult::Intercept(())
}

