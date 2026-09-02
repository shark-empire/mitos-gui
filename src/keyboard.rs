use smithay::backend::input::{Event, InputBackend, KeyboardKeyEvent, KeyState};
use smithay::input::keyboard::{keysyms, FilterResult, Keysym, ModifiersState};
use smithay::utils::SERIAL_COUNTER;

use crate::state::MitosGuiState;

/// Feed one raw keyboard event into the MITOS seat.
pub fn handle_keyboard_key<B: InputBackend>(state: &mut MitosGuiState, event: B::KeyboardKeyEvent) {
    let Some(keyboard) = state.seat.get_keyboard() else { return; };

    let serial = SERIAL_COUNTER.next_serial();
    let time = event.time_msec();
    let keycode = event.key_code();
    let key_state = event.state();

    keyboard.input::<(), _>(state, keycode, key_state, serial, time, |state, mods, sym| {
        let keysym = sym.modified_sym();

        if state.auth.active {
            return handle_auth_input(state, keysym, key_state);
        }

        if key_state == KeyState::Pressed && handle_media_keys(state, keysym) {
            return FilterResult::Intercept(());
        }

        if state.shell.launcher_visible {
            return handle_launcher_input(state, keysym, key_state);
        }

        if mods.logo && key_state == KeyState::Pressed {
            if let Some(intercept) = handle_wm_shortcuts(state, mods, keysym) {
                return intercept;
            }
        }

        FilterResult::Forward
    });
}

fn handle_media_keys(state: &mut MitosGuiState, keysym: Keysym) -> bool {
    match keysym {
        keysyms::KEY_XF86AudioRaiseVolume => {
            state.muted = false;
            state.volume = state.volume.saturating_add(5).min(100);
            state.osd.trigger(crate::state::OsdIcon::Volume, state.volume as f32 / 100.0);
            state.pending_full_redraw = true;
            true
        }
        keysyms::KEY_XF86AudioLowerVolume => {
            state.muted = false;
            state.volume = state.volume.saturating_sub(5);
            state.osd.trigger(crate::state::OsdIcon::Volume, state.volume as f32 / 100.0);
            state.pending_full_redraw = true;
            true
        }
        keysyms::KEY_XF86AudioMute => {
            state.muted = !state.muted;
            let icon = if state.muted { crate::state::OsdIcon::Muted } else { crate::state::OsdIcon::Volume };
            state.osd.trigger(icon, if state.muted { 0.0 } else { state.volume as f32 / 100.0 });
            state.pending_full_redraw = true;
            true
        }
        _ => false,
    }
}

fn handle_wm_shortcuts(
    state: &mut MitosGuiState,
    mods: &ModifiersState,
    keysym: Keysym,
) -> Option<FilterResult<()>> {
    let active_monitor = state.active_output_name();

    // Launcher Toggle (Super + Space)
    if keysym == keysyms::KEY_space {
        state.shell.toggle_launcher();
        state.pending_full_redraw = true;
        return Some(FilterResult::Intercept(()));
    }

    // Diagnostics & Auth
    if mods.shift {
        match keysym {
            keysyms::KEY_N => {
                state.toggle_night_light();
                return Some(FilterResult::Intercept(()));
            }
            keysyms::KEY_A => {
                state.auth.request("MITOS Package Manager", "Install system updates");
                state.pending_full_redraw = true;
                return Some(FilterResult::Intercept(()));
            }
            keysyms::KEY_R => {
                crate::session::reboot();
                return Some(FilterResult::Intercept(()));
            }
            _ => {}
        }
    } else if keysym == keysyms::KEY_n {
        state.notifications.push("MITOS System", "Notification Engine Active", "Stage 6 desktop services are online.");
        state.pending_full_redraw = true;
        return Some(FilterResult::Intercept(()));
    }

    // Window Management
    match keysym {
        keysyms::KEY_q => crate::wm::close_focused(state),
        keysyms::KEY_f => crate::wm::toggle_fullscreen(state),
        keysyms::KEY_Up => crate::wm::toggle_maximize(state),
        keysyms::KEY_Down => {
            if mods.shift { crate::wm::restore_minimized(state) } else { crate::wm::minimize_focused(state) }
        }
        keysyms::KEY_Left => crate::wm::snap(state, crate::wm::SnapSide::Left),
        keysyms::KEY_Right => crate::wm::snap(state, crate::wm::SnapSide::Right),
        keysyms::KEY_Tab => crate::wm::cycle_focus(state),
        keysyms::KEY_Print | keysyms::KEY_Sys_Req => {
            state.pending_screenshot = true;
            state.pending_full_redraw = true;
        }
        // Workspace Switching & Moving
        keysyms::KEY_1 | keysyms::KEY_2 | keysyms::KEY_3 | keysyms::KEY_4 => {
            let target = match keysym {
                keysyms::KEY_1 => 0,
                keysyms::KEY_2 => 1,
                keysyms::KEY_3 => 2,
                keysyms::KEY_4 => 3,
                _ => unreachable!(),
            };

            if mods.shift {
                if let Some(win) = state.focused_window.clone() {
                    crate::wm::meta(&win).workspace = target;
                }
            }
            state.switch_workspace(&active_monitor, target);
        }
        _ => return None, // Not a WM shortcut
    }

    Some(FilterResult::Intercept(()))
}

fn handle_launcher_input(
    state: &mut MitosGuiState,
    keysym: Keysym,
    key_state: KeyState,
) -> FilterResult<()> {
    if key_state != KeyState::Pressed { return FilterResult::Intercept(()); }

    match keysym {
        keysyms::KEY_Escape => state.shell.toggle_launcher(),
        keysyms::KEY_Return => {
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
                state.shell.launcher_selected = (state.shell.launcher_selected + 1) % state.shell.launcher_results.len();
            }
        }
        keysyms::KEY_Up => {
            if !state.shell.launcher_results.is_empty() {
                state.shell.launcher_selected = state.shell.launcher_selected.checked_sub(1).unwrap_or(state.shell.launcher_results.len() - 1);
            }
        }
        _ => {
            if let Some(c) = keysym_to_char(keysym) {
                if c.is_ascii_graphic() || c == ' ' {
                    state.shell.launcher_query.push(c);
                    update_launcher_results(state);
                }
            }
        }
    }

    state.pending_full_redraw = true;
    FilterResult::Intercept(())
}

fn update_launcher_results(state: &mut MitosGuiState) {
    state.shell.launcher_results = crate::shell_interaction::search_apps(&state.launcher_apps, &state.shell.launcher_query);
    state.shell.launcher_selected = 0;
}

pub fn toggle_launcher(state: &mut MitosGuiState) {
    state.shell.toggle_launcher();
    state.pending_full_redraw = true;
}

fn keysym_to_char(keysym: Keysym) -> Option<char> {
    let raw = keysym.raw();
    if (0x20..=0x7E).contains(&raw) {
        char::from_u32(raw)
    } else {
        match raw {
            0xFF0D => Some('\n'),
            0xFF08 => Some('\x08'),
            _ => None,
        }
    }
}

fn handle_auth_input(
    state: &mut MitosGuiState,
    keysym: Keysym,
    key_state: KeyState,
) -> FilterResult<()> {
    if key_state != KeyState::Pressed { return FilterResult::Intercept(()); }

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
