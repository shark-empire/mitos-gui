
//! Keyboard input.
//!
//! Stage 4, 5 & 7 keyboard handling:
//! - Launcher search navigation (when open)
//! - Super + Space toggles the MITOS launcher
//! - Stage 4 window manager shortcuts (close, maximize, snap, etc.)
//! - Stage 7 OSD (On-Screen Display) for media keys
//! - Stage 7 Night Light toggle
//! - Stage 7 sticky/slow/bounce keys (see `accessibility.rs` for what
//!   these do and don't cover)
//! - Forward normal keys to the focused Wayland client
use smithay::input::keyboard::Keysym;
use smithay::backend::input::{
    Event,
    InputBackend,
    KeyboardKeyEvent,
    KeyState,
};

use smithay::input::keyboard::{
    keysyms,
    FilterResult,
    KeyboardHandle,
};

use smithay::utils::{Serial, SERIAL_COUNTER};

use crate::state::MitosGuiState;

fn keysym_to_char(keysym: Keysym) -> Option<char> {
    let raw = keysym.raw();
    if (0x20..=0x7E).contains(&raw) {
        char::from_u32(raw)
    } else if raw == 0xFF0D { // Enter
        Some('\n')
    } else if raw == 0xFF08 { // Backspace
        Some('\x08')
    } else {
        None
    }
}

/// Feed one raw keyboard event into the MITOS seat.
///
/// Slow-keys/bounce-keys sit in front of everything else here (see
/// `accessibility::KeyFilter`): they decide whether this physical
/// event reaches the seat's keyboard state *at all*, before anything
/// downstream -- including xkb's own modifier tracking -- ever sees
/// it.
pub fn handle_keyboard_key<B: InputBackend>(
    state: &mut MitosGuiState,
    event: B::KeyboardKeyEvent,
) {
    let Some(keyboard) = state.seat.get_keyboard() else {
        return;
    };

    let time = event.time_msec();
    let keycode = event.key_code();
    let key_state = event.state();

    let decision = state.key_filter.filter(
        keycode,
        key_state == KeyState::Pressed,
        state.home_screen.slow_keys,
        state.home_screen.bounce_keys,
    );

    match decision {
        crate::accessibility::FilterDecision::Swallow => {}

        crate::accessibility::FilterDecision::Pass => {
            let serial = SERIAL_COUNTER.next_serial();
            dispatch_key(state, &keyboard, keycode, key_state, serial, time);
        }

        crate::accessibility::FilterDecision::ConfirmedHold => {
            // Slow-keys swallowed the original keydown while timing
            // it, so nothing downstream (including xkb) has seen this
            // key yet. Send a fresh, properly-paired press then
            // release now instead -- both timestamped to this moment,
            // not the original keydown -- so xkb's internal state
            // machine sees an ordinary complete keypress rather than a
            // resurrected half of one.
            let serial = SERIAL_COUNTER.next_serial();
            dispatch_key(state, &keyboard, keycode, KeyState::Pressed, serial, time);
            let serial = SERIAL_COUNTER.next_serial();
            dispatch_key(state, &keyboard, keycode, KeyState::Released, serial, time);
        }
    }
}

fn dispatch_key(
    state: &mut MitosGuiState,
    keyboard: &KeyboardHandle<MitosGuiState>,
    keycode: smithay::backend::input::Keycode,
    key_state: KeyState,
    serial: Serial,
    time: u32,
) {
    keyboard.input::<(), _>(
        state,
        keycode,
        key_state,
        serial,
        time,
        |state, mods, sym| {
            let keysym = sym.modified_sym();

            // --------------------------------------------------------
            // STICKY KEYS: bare-modifier-tap detection.
            //
            // Runs unconditionally, ahead of every other branch below
            // (including the auth-prompt gate), since it only ever
            // touches `state.sticky_latch`/`state.sticky_combo_used`,
            // ready for whatever key comes next -- it never itself
            // intercepts or forwards this event.
            //
            // `any_mod_down` reflects the *post*-this-event modifier
            // state (xkb has already applied this press/release by the
            // time this closure runs), so on the release that ends a
            // bare tap it correctly reads as "nothing held anymore".
            // --------------------------------------------------------
            let is_mod_key = crate::accessibility::StickyLatch::is_modifier_keysym(keysym.raw());
            let any_mod_down = mods.shift || mods.ctrl || mods.alt || mods.logo;

            if key_state == KeyState::Pressed && !is_mod_key && any_mod_down {
                state.sticky_combo_used = true;
            }

            if is_mod_key && key_state == KeyState::Released && state.home_screen.sticky_keys {
                if !any_mod_down && !state.sticky_combo_used {
                    state.sticky_latch.toggle(keysym.raw());
                    state.pending_full_redraw = true;
                }
            }

            if !any_mod_down {
                state.sticky_combo_used = false;
            }

            // --------------------------------------------------------
            // TOGGLE KEY FEEDBACK (Stage 7)
            // Caps/Num Lock have no on-screen indicator of their own on
            // most keyboards worth relying on -- surface a change via
            // the same OSD the volume/brightness keys already use.
            // --------------------------------------------------------
            if mods.caps_lock != state.caps_lock_was {
                state.caps_lock_was = mods.caps_lock;
                state.osd.trigger(
                    crate::state::OsdIcon::CapsLock,
                    if mods.caps_lock { 1.0 } else { 0.0 },
                );
                state.pending_full_redraw = true;
            }
            if mods.num_lock != state.num_lock_was {
                state.num_lock_was = mods.num_lock;
                state.osd.trigger(
                    crate::state::OsdIcon::NumLock,
                    if mods.num_lock { 1.0 } else { 0.0 },
                );
                state.pending_full_redraw = true;
            }

            // Every check below this point sees physically-held OR
            // sticky-latched modifiers under the name `mods`, without
            // needing to touch each individual check -- see
            // `accessibility.rs`'s module doc for exactly what that
            // does and doesn't extend to.
            let eff_mods = crate::accessibility::EffectiveMods::compute(mods, &state.sticky_latch);
            let mods = &eff_mods;

            // --------------------------------------------------------
            // SECURE AUTHENTICATION PROMPT (Highest Priority)
            // --------------------------------------------------------
            if state.auth.active {
                return handle_auth_input(state, keysym.into(), key_state);
            }

            // --------------------------------------------------------
            // Stage 6 & 7: hardware media keys & OSD
            // --------------------------------------------------------
            if key_state == KeyState::Pressed {
                match keysym.raw() {
                    keysyms::KEY_XF86AudioRaiseVolume => {
                        state.muted = false;
                        state.volume = state.volume.saturating_add(5).min(100);
                        crate::audio::set_muted(false);
                        crate::audio::set_volume(state.volume);
                        state.osd.trigger(crate::state::OsdIcon::Volume, state.volume as f32 / 100.0);
                        state.pending_full_redraw = true;
                        return FilterResult::Intercept(());
                    }

                    keysyms::KEY_XF86AudioLowerVolume => {
                        state.muted = false;
                        state.volume = state.volume.saturating_sub(5);
                        crate::audio::set_muted(false);
                        crate::audio::set_volume(state.volume);
                        state.osd.trigger(crate::state::OsdIcon::Volume, state.volume as f32 / 100.0);
                        state.pending_full_redraw = true;
                        return FilterResult::Intercept(());
                    }

                    keysyms::KEY_XF86AudioMute => {
                        state.muted = !state.muted;
                        crate::audio::set_muted(state.muted);
                        let icon = if state.muted { crate::state::OsdIcon::Muted } else { crate::state::OsdIcon::Volume };
                        state.osd.trigger(icon, if state.muted { 0.0 } else { state.volume as f32 / 100.0 });
                        state.pending_full_redraw = true;
                        return FilterResult::Intercept(());
                    }

                    keysyms::KEY_XF86MonBrightnessUp => {
                        state.brightness = state.brightness.saturating_add(5).min(100);
                        crate::brightness::set_brightness(state.brightness);
                        state.osd.trigger(crate::state::OsdIcon::Brightness, state.brightness as f32 / 100.0);
                        state.pending_full_redraw = true;
                        return FilterResult::Intercept(());
                    }

                    keysyms::KEY_XF86MonBrightnessDown => {
                        state.brightness = state.brightness.saturating_sub(5);
                        crate::brightness::set_brightness(state.brightness);
                        state.osd.trigger(crate::state::OsdIcon::Brightness, state.brightness as f32 / 100.0);
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
                return handle_launcher_input(state, keysym.into(), key_state);
            }

            // --------------------------------------------------------
            // DOCK KEYBOARD NAVIGATION (Stage 7)
            // While focused, captures Left/Right/Enter/Escape; mirrors
            // how the launcher results list captures input above.
            // --------------------------------------------------------
            if state.shell.dock_focused.is_some() {
                return handle_dock_focus_input(state, keysym.into(), key_state);
            }

            // Super + D: move keyboard focus into the dock, so it's
            // reachable without a mouse. Only makes sense with a dock
            // to focus into.
            if mods.logo && keysym == keysyms::KEY_d.into() && state.shell.dock.is_some() {
                state.shell.toggle_dock_focus();
                state.pending_full_redraw = true;
                tracing::info!(
                    "MITOS: dock focus {}",
                    if state.shell.dock_focused.is_some() { "entered" } else { "exited" }
                );
                return FilterResult::Intercept(());
            }

            // Super + K: toggle the on-screen keyboard.
            if mods.logo && keysym == keysyms::KEY_k.into() {
                state.osk_visible = !state.osk_visible;
                state.pending_full_redraw = true;
                return FilterResult::Intercept(());
            }

            // --------------------------------------------------------
            // MITOS launcher shortcut
            //
            // Super + Space
            // --------------------------------------------------------
            if mods.logo && keysym == keysyms::KEY_space.into() {
                toggle_launcher(state);

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
            // Stage 4 & 7 Window Manager & Desktop shortcuts
            // --------------------------------------------------------
            if mods.logo {

                // Super + N: Push a test notification (Stage 6)
                if !mods.shift && keysym == keysyms::KEY_n.into() {
                    state.notifications.push(
                        "MITOS System",
                        "Notification Engine Active",
                        "Stage 6 desktop services are online.",
                    );
                    state.pending_full_redraw = true;
                    return FilterResult::Intercept(());
                }

                // Super + Shift + N: Toggle Night Light (Stage 7)
                if mods.shift && keysym == keysyms::KEY_n.into() {
                    state.toggle_night_light();
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

                // Super + Shift + P: Power off via mitos-init
                if mods.shift && keysym == keysyms::KEY_p.into() {
                    crate::session::poweroff();
                    return FilterResult::Intercept(());
                }

                // Super + Shift + H: Halt via mitos-init
                if mods.shift && keysym == keysyms::KEY_h.into() {
                    crate::session::halt();
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

                 // Super + PrintScreen: Screenshot
                if keysym == keysyms::KEY_Print.into() || keysym == keysyms::KEY_Sys_Req.into() {
                    state.pending_screenshot = true;
                    state.pending_full_redraw = true;
                    return FilterResult::Intercept(());
                }
                                // Super + L: ask mitos-session to lock the session.
                // mitos-session decides yes/no (e.g. no password set =>
                // refused); the refusal arrives back through
                // poll_session_ipc() as Response::Error and becomes a toast.
                if !mods.shift && keysym == keysyms::KEY_l.into() {
                    match state.session_ipc.as_mut() {
                        Some(ipc) => {
                            let session_id = ipc.session_id;
                            ipc.send(&mitos_session::ipc::Request::LockSession { session_id });
                        }
                        None => {
                            // Standalone/dev run: no mitos-session to lock us.
                            state.notifications.push(
                                "MITOS Security",
                                "Cannot lock screen",
                                "Not connected to mitos-session.",
                            );
                            state.pending_full_redraw = true;
                        }
                    }
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

                // Super + 1/2/3/4: Switch Workspace (on the monitor under the pointer)
                if !mods.shift {
                    let target = match keysym.raw() {
                        keysyms::KEY_1 => Some(0),
                        keysyms::KEY_2 => Some(1),
                        keysyms::KEY_3 => Some(2),
                        keysyms::KEY_4 => Some(3),
                        _ => None,
                    };
                    if let Some(ws) = target {
                        let active = state.active_output_name();
                        state.switch_workspace(&active, ws);
                        return FilterResult::Intercept(());
                    }
                }

                // Super + Shift + 1/2/3/4: Move focused window to Workspace
                if mods.shift {
                    if let Some(win) = state.focused_window.clone() {
                        let target = match keysym.raw() {
                            keysyms::KEY_1 => Some(0),
                            keysyms::KEY_2 => Some(1),
                            keysyms::KEY_3 => Some(2),
                            keysyms::KEY_4 => Some(3),
                            _ => None,
                        };
                        if let Some(t) = target {
                            let active = state.active_output_name();
                            crate::wm::meta(&win).workspace.insert(active.clone(), t);
                            state.switch_workspace(&active, t); // Follow the window
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
/// Handles Left/Right/Enter/Escape while the dock has keyboard focus
/// (Stage 7 dock keyboard navigation -- see `MitosShell::dock_focused`).
fn handle_dock_focus_input(
    state: &mut MitosGuiState,
    keysym: Keysym,
    key_state: KeyState,
) -> FilterResult<()> {
    if key_state != KeyState::Pressed {
        return FilterResult::Intercept(());
    }

    match keysym.raw() {
        keysyms::KEY_Escape => {
            state.shell.dock_focused = None;
        }

        keysyms::KEY_Left => state.shell.move_dock_focus(-1),
        keysyms::KEY_Right => state.shell.move_dock_focus(1),

        keysyms::KEY_Return | keysyms::KEY_space => {
            let id = state
                .shell
                .dock_focused
                .and_then(|i| state.shell.dock_layout.items.get(i))
                .map(|item| item.id);

            if let Some(id) = id {
                crate::shell_interaction::launch_app(state, id);
            }
            state.shell.dock_focused = None;
        }

        _ => {}
    }

    state.pending_full_redraw = true;
    FilterResult::Intercept(())
}

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
            if let Some(c) = keysym_to_char(keysym.into()) {
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
            if state.auth.request_id.is_some() {
                cancel_elevation(state);
            } else {
                state.auth.cancel();
                if !state.auth.is_lock_screen {
                    state.notifications.push("MITOS Security", "Authentication cancelled", "");
                }
            }
        }
        keysyms::KEY_Return => {
            if state.auth.is_lock_screen {
                submit_lock_screen(state);
            } else if state.auth.request_id.is_some() {
                submit_elevation(state);
            } else if state.auth.submit() {
                state.notifications.push("MITOS Security", "Authentication successful", "Privileges granted.");
            } else {
                state.notifications.push("MITOS Security", "Authentication failed", "Incorrect password.");
            }
        }
        keysyms::KEY_BackSpace => {
            state.auth.password.pop();
        }
        _ => {
            if let Some(c) = keysym_to_char(keysym.into()) {
                if c.is_ascii_graphic() || c == ' ' {
                    state.auth.password.push(c);
                }
            }
        }
    }

    state.pending_full_redraw = true;
    FilterResult::Intercept(())
}

/// Send the typed password to mitos-session as a real `Unlock`
/// request instead of checking it locally -- mitos-session is the
/// only side that ever decides whether a lock-screen password is
/// correct (see `docs/security.md` in that project).
fn submit_lock_screen(state: &mut MitosGuiState) {
    if state.auth.pending {
        return;
    }

    let Some(ipc) = state.session_ipc.as_mut() else {
        // No connection to mitos-session at all (e.g. running mitos-gui
        // standalone for development) -- nothing to check the password
        // against, so don't pretend either way.
        state.auth.error_msg = Some("Not connected to mitos-session".to_string());
        return;
    };

    let session_id = ipc.session_id;
    let user_name = std::env::var("USER").unwrap_or_default();
    let password = std::mem::take(&mut state.auth.password);

    state.auth.pending = true;
    ipc.send(&mitos_session::ipc::Request::Unlock { session_id, user_name, password });
}

/// Answer the currently-shown elevation prompt with `response`, then
/// wait for mitos-session's own `HideElevationPrompt` to actually
/// close it (`poll_session_ipc`) rather than closing locally -- same
/// "mitos-session decides, mitos-gui only reacts" rule the lock screen
/// follows, and it means a cancel that arrived from somewhere else
/// (e.g. root running `mitos-sessionctl`) and our own cancel here end
/// up going through exactly one code path.
fn respond_elevation(state: &mut MitosGuiState, response: mitos_session::elevation::ElevationResponse) {
    if state.auth.pending {
        return;
    }
    let Some(request_id) = state.auth.request_id else { return };

    let Some(ipc) = state.session_ipc.as_mut() else {
        state.auth.error_msg = Some("Not connected to mitos-session".to_string());
        return;
    };

    state.auth.pending = true;
    ipc.send(&mitos_session::ipc::Request::RespondElevation { request_id, response });
}

/// Send the typed password for the active elevation prompt -- mirrors
/// `submit_lock_screen`, just against `RespondElevation` instead of
/// `Unlock`. Checks `pending` before touching `password`, not after:
/// otherwise a second Enter mashed while the first submission is
/// still in flight would silently discard whatever's been typed since
/// (`respond_elevation`'s own `pending` guard is too late for that --
/// by the time it runs, the password this call read would already be
/// gone either way).
fn submit_elevation(state: &mut MitosGuiState) {
    if state.auth.pending {
        return;
    }
    let password = std::mem::take(&mut state.auth.password);
    respond_elevation(state, mitos_session::elevation::ElevationResponse::Password(password));
}

/// Decline the active elevation prompt. Unlike the lock screen,
/// elevation prompts *can* be turned down -- mitos-service just gets
/// told no, the same as if the password had been wrong every time
/// until it gave up asking.
fn cancel_elevation(state: &mut MitosGuiState) {
    state.auth.password.clear();
    respond_elevation(state, mitos_session::elevation::ElevationResponse::Cancelled);
}

/// Route one on-screen-keyboard key click (see `osk.rs`'s module doc
/// for the two delivery paths and their confidence levels). Mirrors
/// `handle_auth_input`/`handle_launcher_input`'s character handling
/// exactly -- same fields, same follow-up calls -- when one of
/// MITOS's own text fields is active; synthesizes a real key event
/// otherwise.
pub fn handle_osk_key(state: &mut MitosGuiState, key: &crate::osk::OskKey) {
    use crate::osk::OskAction;

    if key.action == OskAction::Shift {
        state.osk_shift = !state.osk_shift;
        state.pending_full_redraw = true;
        return;
    }

    // Shift (if it was on) applies to this one key only, regardless of
    // which path below ends up handling it.
    let shift_active = state.osk_shift;
    state.osk_shift = false;

    if state.auth.active {
        match key.action {
            OskAction::Letter => {
                if let Some(c) = crate::osk::key_char(key, shift_active) {
                    state.auth.password.push(c);
                }
            }
            OskAction::Space => state.auth.password.push(' '),
            OskAction::Backspace => {
                state.auth.password.pop();
            }
            OskAction::Enter => {
                if state.auth.is_lock_screen {
                    submit_lock_screen(state);
                } else if state.auth.request_id.is_some() {
                    submit_elevation(state);
                } else if state.auth.submit() {
                    state.notifications.push("MITOS Security", "Authentication successful", "Privileges granted.");
                } else {
                    state.notifications.push("MITOS Security", "Authentication failed", "Incorrect password.");
                }
            }
            OskAction::Shift => unreachable!(),
        }
        state.pending_full_redraw = true;
        return;
    }

    if state.shell.launcher_visible {
        match key.action {
            OskAction::Letter => {
                if let Some(c) = crate::osk::key_char(key, shift_active) {
                    state.shell.launcher_query.push(c);
                    update_launcher_results(state);
                }
            }
            OskAction::Space => {
                state.shell.launcher_query.push(' ');
                update_launcher_results(state);
            }
            OskAction::Backspace => {
                state.shell.launcher_query.pop();
                update_launcher_results(state);
            }
            OskAction::Enter => {
                if let Some(app) = state.shell.launcher_results.get(state.shell.launcher_selected) {
                    crate::shell_interaction::launch_app_entry(app);
                }
                state.shell.toggle_launcher();
            }
            OskAction::Shift => unreachable!(),
        }
        state.pending_full_redraw = true;
        return;
    }

    dispatch_synthetic_key(state, key, shift_active);
}

/// Synthesize a real key event toward whatever client has focus, for
/// on-screen-keyboard clicks when neither the auth prompt nor the
/// launcher is open to receive a character directly. See `osk.rs`'s
/// module doc: this is the lower-confidence of the OSK's two delivery
/// paths.
fn dispatch_synthetic_key(state: &mut MitosGuiState, key: &crate::osk::OskKey, shift_active: bool) {
    let Some(keyboard) = state.seat.get_keyboard() else {
        return;
    };

    let evdev_code = match key.action {
        crate::osk::OskAction::Letter => key.evdev_code,
        crate::osk::OskAction::Space => Some(57),     // KEY_SPACE
        crate::osk::OskAction::Backspace => Some(14), // KEY_BACKSPACE
        crate::osk::OskAction::Enter => Some(28),     // KEY_ENTER
        crate::osk::OskAction::Shift => None,
    };
    let Some(evdev_code) = evdev_code else {
        return;
    };

    // No real hardware timestamp for a synthetic event -- wall-clock
    // millis is a reasonable stand-in, same idea as `ambient_pulse`'s
    // use of wall-clock time elsewhere in this codebase.
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u32)
        .unwrap_or(0);

    const KEY_LEFTSHIFT_EVDEV: u32 = 42;
    // xkb/X11 keycode numbering is the evdev code plus a fixed 8-code
    // offset -- a decades-stable convention, not something that varies
    // by smithay version.
    let to_keycode = |evdev: u32| smithay::backend::input::Keycode::from(evdev + 8);

    if shift_active {
        let s = SERIAL_COUNTER.next_serial();
        dispatch_key(state, &keyboard, to_keycode(KEY_LEFTSHIFT_EVDEV), KeyState::Pressed, s, time);
    }

    let s = SERIAL_COUNTER.next_serial();
    dispatch_key(state, &keyboard, to_keycode(evdev_code), KeyState::Pressed, s, time);
    let s = SERIAL_COUNTER.next_serial();
    dispatch_key(state, &keyboard, to_keycode(evdev_code), KeyState::Released, s, time);

    if shift_active {
        let s = SERIAL_COUNTER.next_serial();
        dispatch_key(state, &keyboard, to_keycode(KEY_LEFTSHIFT_EVDEV), KeyState::Released, s, time);
    }

    state.pending_full_redraw = true;
}
