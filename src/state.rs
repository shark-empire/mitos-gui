//! Global state for the MITOS compositor.

use crate::desktop::HomeScreenConfig;
use crate::renderer::GlassPanel;
use crate::shell_interaction::AppEntry;
use crate::wm::InteractiveAction;

use std::collections::HashMap;
use std::time::{Duration, Instant};
use smithay::{
    desktop::{PopupManager, Space, Window},
    input::{
        pointer::CursorImageStatus,
        Seat,
        SeatHandler,
        SeatState,
    },
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Clock, Logical, Monotonic, Point},
    wayland::{
        compositor::CompositorState,
        shell::xdg::XdgShellState,
        shm::ShmState,
    },
};

// ============================================================================
// ON-SCREEN DISPLAY (OSD)
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OsdIcon {
    Volume,
    Muted,
    Brightness,
    /// Stage 7: toggle-key feedback -- lets someone who can't see the
    /// keyboard's own indicator LED confirm Caps/Num Lock changed.
    CapsLock,
    NumLock,
}

#[derive(Clone, Debug)]
pub struct OsdState {
    pub active: bool,
    pub icon: OsdIcon,
    pub value: f32, // 0.0 to 1.0
    pub last_updated: Instant,
}

impl OsdState {
    pub fn new() -> Self {
        Self {
            active: false,
            icon: OsdIcon::Volume,
            value: 0.0,
            // Hide immediately on startup by setting it to the past
            last_updated: Instant::now() - Duration::from_secs(3),
        }
    }

    pub fn trigger(&mut self, icon: OsdIcon, value: f32) {
        self.active = true;
        self.icon = icon;
        self.value = value.clamp(0.0, 1.0);
        self.last_updated = Instant::now();
    }
}

/// Fixed number of virtual desktops per monitor.
pub const WORKSPACE_COUNT: usize = 4;

// ============================================================================
// MITOS SHELL
// ============================================================================

/// State owned by the MITOS visual shell.
///
/// The renderer is responsible for drawing these components.
/// This struct is responsible only for describing their state and geometry.
#[derive(Debug)]
pub struct MitosShell {
    /// Main MITOS top bar.
    pub top_bar: Option<GlassPanel>,

    /// Application launcher panel.
    pub launcher: Option<GlassPanel>,

    /// Desktop dock.
    pub dock: Option<GlassPanel>,

    pub dock_layout: crate::desktop::DockLayout,

    /// Whether the launcher is currently visible.
    pub launcher_visible: bool,

    /// Set the moment the launcher opens; drives the one-shot "activation
    /// ring" pulse in `collect_shell_elements`/`create_launcher_ring_element`.
    /// Left alone on close — only the open transition animates.
    pub launcher_anim: Option<crate::animation::Animation>,

    // --- Launcher Search State ---
    pub launcher_query: String,
    pub launcher_results: Vec<AppEntry>,
    pub launcher_selected: usize,

    /// Stage 7: which dock item has *keyboard* focus (distinct from
    /// mouse hover/magnification), when dock keyboard navigation is
    /// active. `None` means the dock isn't currently keyboard-focused.
    pub dock_focused: Option<usize>,
}

impl MitosShell {
    pub fn new() -> Self {
        Self {
            top_bar: None,
            launcher: None,
            dock: None,
            dock_layout: crate::desktop::DockLayout::default(),
            launcher_visible: false,
            launcher_anim: None,
            launcher_query: String::new(),
            launcher_results: Vec::new(),
            launcher_selected: 0,
            dock_focused: None,
        }
    }

    /// Recalculate shell geometry for the current output.
    pub fn update_layout(
        &mut self,
        config: &HomeScreenConfig,
        output_size: smithay::utils::Size<i32, smithay::utils::Logical>,
    ) {
        let layout = crate::desktop::ShellLayout::calculate(config, output_size);

        self.top_bar = layout.top_bar;
        self.launcher = layout.launcher;
        self.dock = layout.dock;
    }

    /// Toggle the launcher visibility.
    pub fn toggle_launcher(&mut self) {
        self.launcher_visible = !self.launcher_visible;
        if self.launcher_visible {
            // Reset state when opening
            self.launcher_query.clear();
            self.launcher_results = crate::shell_interaction::discover_apps();
            self.launcher_selected = 0;
            self.launcher_anim = Some(crate::animation::Animation::new(Duration::from_millis(420)));
        }
    }

    /// Toggle keyboard focus into/out of the dock (Stage 7: makes the
    /// dock reachable without a mouse, matching how the launcher's
    /// results list already is).
    pub fn toggle_dock_focus(&mut self) {
        self.dock_focused = match self.dock_focused {
            Some(_) => None,
            None => Some(0),
        };
    }

    /// Move dock keyboard focus by `delta` items, wrapping at the ends.
    /// No-op if the dock isn't currently keyboard-focused or is empty.
    pub fn move_dock_focus(&mut self, delta: i32) {
        let len = self.dock_layout.items.len();
        if len == 0 {
            return;
        }
        if let Some(i) = self.dock_focused {
            let next = (i as i32 + delta).rem_euclid(len as i32) as usize;
            self.dock_focused = Some(next);
        }
    }
}

// ============================================================================
// GLOBAL COMPOSITOR STATE
// ============================================================================

/// Global state for the MITOS compositor.
pub struct MitosGuiState {
    // ------------------------------------------------------------------------
    // Wayland protocol state
    // ------------------------------------------------------------------------
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,

    // ------------------------------------------------------------------------
    // Input
    // ------------------------------------------------------------------------
    pub seat_state: SeatState<Self>,
    pub seat: Seat<Self>,

    pub current_workspace: HashMap<String, usize>,

    /// Cross-workspace swipe-gesture offset, in units of one workspace width.
    pub workspace_swipe_x: f64,

    // ------------------------------------------------------------------------
    // MITOS shell
    // ------------------------------------------------------------------------
    pub shell: MitosShell,

    // ------------------------------------------------------------------------
    // Output
    // ------------------------------------------------------------------------

     pub outputs: Vec<Output>,


    // ------------------------------------------------------------------------
    // Desktop
    // ------------------------------------------------------------------------
    pub home_screen: HomeScreenConfig,
    pub space: Space<Window>,
    pub popups: PopupManager,

    // ------------------------------------------------------------------------
    // Pointer / timing
    // ------------------------------------------------------------------------
    pub pointer_location: Point<f64, Logical>,
    pub clock: Clock<Monotonic>,
    pub focused_window: Option<Window>,
    pub minimized: Vec<Window>,
    pub interactive: Option<InteractiveAction>,

    // ------------------------------------------------------------------------
    // Stage 6: System Services
    // ------------------------------------------------------------------------
    pub notifications: crate::notifications::NotificationManager,
    pub network: crate::status::NetworkStatus,
    pub battery: Option<crate::status::BatteryStatus>,
    pub volume: u8,
    pub muted: bool,
    pub brightness: u8,
    pub last_status_poll: Instant,

    /// Set by the DRM vblank handler; consumed by the DRM main loop.
    pub drm_vblank: bool,

    /// Discovered applications for the launcher.
    pub launcher_apps: Vec<AppEntry>,

    /// Set by the config watcher; consumed by the main loop to force
    /// a full redraw and shader recompile after a live config reload.
    pub pending_full_redraw: bool,

    /// Stage 6: Secure authentication prompt.
    pub auth: crate::auth::AuthPrompt,

    // ------------------------------------------------------------------------
    // Stage 7: OSD, Night Light, Hot Corners
    // ------------------------------------------------------------------------
    pub osd: OsdState,
    pub night_light: bool,
    /// Drives the night-light tint's fade in/out; restarted every time
    /// `toggle_night_light` flips the setting. A zero-duration `Animation`
    /// (its initial value, and its state right after any transition
    /// finishes) reads as "already settled" -- see `Animation::progress`.
    pub night_light_anim: crate::animation::Animation,
    pub hot_corners_last_triggered: Instant,

    // ------------------------------------------------------------
    // Accessibility (Stage 7)
    // ------------------------------------------------------------
    /// Slow-keys/bounce-keys timing state -- see `accessibility.rs`.
    pub key_filter: crate::accessibility::KeyFilter<smithay::backend::input::Keycode>,
    /// Sticky-keys: which modifiers are currently latched from a bare
    /// tap, waiting for the next non-modifier key.
    pub sticky_latch: crate::accessibility::StickyLatch,
    /// Sticky-keys bookkeeping: whether a non-modifier key has been
    /// pressed since the modifiers currently held all went down bare
    /// (i.e. whether this is shaping up to be a chord, not a tap).
    pub sticky_combo_used: bool,
    /// Last known Caps/Num Lock state, to detect toggles for OSD
    /// feedback -- see the top of `keyboard.rs`'s dispatch closure.
    pub caps_lock_was: bool,
    pub num_lock_was: bool,

    /// Stage 7: on-screen keyboard visibility and its own Shift layer
    /// (separate from the physical keyboard's -- see `osk.rs`).
    pub osk_visible: bool,
    pub osk_shift: bool,
    /// Stage 7: last-seen launcher/auth visibility, so
    /// `refresh_accessibility_summary` can tell a transition (worth an
    /// AT-SPI `StateChanged` announcement) from "still open this frame
    /// too" (not worth re-announcing every frame).
    atspi_launcher_was_visible: bool,
    atspi_auth_was_active: bool,
    pub pending_screenshot: bool,
    pub dbus_service: Option<crate::dbus::DbusService>,
    /// Stage 7: AT-SPI provider for MITOS's own shell -- `None` when
    /// no accessibility bus is available (normal on most systems; see
    /// `atspi.rs`'s module doc), not an error condition.
    pub atspi: Option<crate::atspi::AtspiProvider>,

    /// Connection to mitos-session for lock-screen/session IPC. `None`
    /// when mitos-gui isn't running under a real mitos-session-managed
    /// session (e.g. launched by hand for development) -- lock-screen
    /// support is simply unavailable in that case.
    pub session_ipc: Option<crate::session_ipc::SessionIpc>,

    /// Throttle for `report_activity()` -- mitos-session's idle/lock
    /// timers just need a periodic "still here", not a message per
    /// input event.
    pub last_activity_report: Instant,

    /// Clipboard/selection + drag-and-drop protocol state (`wl_data_device_manager`).
    pub data_device_state: smithay::wayland::selection::data_device::DataDeviceState,

}

impl MitosGuiState {
    /// Create the initial MITOS compositor state.
    pub fn new(
        compositor_state: CompositorState,
        xdg_shell_state: XdgShellState,
        shm_state: ShmState,
        seat_state: SeatState<Self>,
        seat: Seat<Self>,
        outputs: Vec<Output>,
        home_screen: HomeScreenConfig,
        dbus_service: crate::dbus::DbusService, // Added
        session_ipc: Option<crate::session_ipc::SessionIpc>,
        data_device_state: smithay::wayland::selection::data_device::DataDeviceState,
    ) -> Self {
        // ------------------------------------------------------------
        // Desktop space
        // ------------------------------------------------------------
        let mut space = Space::default();
        let mut offset_x = 0;
        for o in &outputs {
            space.map_output(o, (offset_x, 0));
            if let Some(mode) = o.current_mode() {
                offset_x += mode.size.w;
            }
        }

     // Initialize the HashMap
        let mut current_workspace = HashMap::new();
        for o in &outputs {
            current_workspace.insert(o.name(), 0);
        }

        let mut shell = MitosShell::new();
        let output_size = outputs
            .first()
            .and_then(|o| o.current_mode())
            .map(|mode| {
                smithay::utils::Size::<i32, smithay::utils::Logical>::from((
                    mode.size.w,
                    mode.size.h,
                ))
            })
            .unwrap_or_else(|| {
                smithay::utils::Size::<i32, smithay::utils::Logical>::new(1280, 720)
            });

        shell.update_layout(&home_screen, output_size);

        // ------------------------------------------------------------
        // Pre-discover apps for the launcher search
        // ------------------------------------------------------------
        let launcher_apps = crate::shell_interaction::discover_apps();
        
        // Pre-populate launcher results so it's ready immediately when opened
        shell.launcher_results = launcher_apps.clone();

        let initial_night_light = home_screen.night_light;

        // ------------------------------------------------------------
        // Global state
        // ------------------------------------------------------------
        Self {
            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
            seat,
            current_workspace,
            workspace_swipe_x: 0.0,
            shell,
            outputs,
            home_screen,
            space,
            popups: PopupManager::default(),
            pointer_location: (0.0, 0.0).into(),
            clock: Clock::new(),
            focused_window: None,
            minimized: Vec::new(),
            interactive: None,
            
            // Stage 6 Services
            notifications: crate::notifications::NotificationManager::new(),
            network: crate::status::NetworkStatus::Offline,
            battery: None,
            volume: 70,
            muted: false,
            brightness: 100,
            last_status_poll: Instant::now(),
            drm_vblank: false,
            auth: crate::auth::AuthPrompt::new(),
            launcher_apps,
            pending_full_redraw: false,

            // Stage 7 Features
            osd: OsdState::new(),
            night_light: initial_night_light,
            night_light_anim: crate::animation::Animation::new(Duration::ZERO),
            hot_corners_last_triggered: Instant::now() - Duration::from_secs(1),

            key_filter: crate::accessibility::KeyFilter::default(),
            sticky_latch: crate::accessibility::StickyLatch::default(),
            sticky_combo_used: false,
            caps_lock_was: false,
            num_lock_was: false,

            osk_visible: home_screen.on_screen_keyboard,
            osk_shift: false,
            atspi_launcher_was_visible: false,
            atspi_auth_was_active: false,

            pending_screenshot: false,
            dbus_service: Some(dbus_service),
            atspi: crate::atspi::AtspiProvider::connect(
                crate::desktop::DockLayout::default()
                    .items
                    .into_iter()
                    .map(|item| crate::atspi::DockItemInfo { id: item.id, name: item.name })
                    .collect(),
            ),
            session_ipc,
            last_activity_report: Instant::now(),
            data_device_state,
        }
    }

    /// Toggle the Night Light (Eye Comfort) mode.
    pub fn toggle_night_light(&mut self) {
        self.night_light = !self.night_light;
        self.night_light_anim = crate::animation::Animation::new(
            Duration::from_millis(crate::theme::MitosTheme::ANIMATION_MS),
        );
        self.pending_full_redraw = true;
        println!("MITOS GUI: Night Light {}", if self.night_light { "enabled" } else { "disabled" });
    }

    /// Re-poll kernel status every 5 seconds.
    /// Returns true if anything changed (triggers a redraw).
    pub fn poll_status_if_due(&mut self) -> bool {
        if self.last_status_poll.elapsed() < Duration::from_secs(5) {
            return false;
        }

        self.last_status_poll = Instant::now();

        let net = crate::status::poll_network();
        let bat = crate::status::poll_battery();

        let changed = net != self.network || bat != self.battery;

        self.network = net;
        self.battery = bat;

        changed
    }

    /// Reload ~/.config/mitos/home.conf and recompute the shell.
    pub fn reload_configuration(&mut self) {
        println!("MITOS GUI: home.conf changed, reloading configuration");

        let old_config = self.home_screen.clone();
        self.home_screen = HomeScreenConfig::load();
        
        // Sync night light state if it was changed externally by mitos-settings
        if self.home_screen.night_light != old_config.night_light {
            self.night_light = self.home_screen.night_light;
            self.night_light_anim = crate::animation::Animation::new(
                Duration::from_millis(crate::theme::MitosTheme::ANIMATION_MS),
            );
        }

        crate::theme::MitosTheme::apply_runtime(&self.home_screen);

        let output_size = crate::wm::output_size(self);

        self.shell.update_layout(&self.home_screen, output_size);
        self.pending_full_redraw = true;
    }

// In impl MitosGuiState

pub fn add_output(&mut self, output: Output) {
    let mut offset_x = 0;
    for o in &self.outputs {
        if let Some(mode) = o.current_mode() {
            offset_x += mode.size.w;
        }
    }
    self.space.map_output(&output, (offset_x, 0));
            
    // Start new monitors on workspace 0
     self.current_workspace.entry(output.name()).or_insert(0);
    self.outputs.push(output);
}

pub fn remove_output(&mut self, output: &Output) {
    self.space.unmap_output(output);
    self.outputs.retain(|o| o != output);

     // Clean up workspace tracking for the removed monitor
    self.current_workspace.remove(&output.name());
    
    // If we removed the primary output, try to focus a window on the remaining one
    if self.outputs.is_empty() {
        self.focused_window = None;
    } else if let Some(focused) = &self.focused_window {
        // Check if focused window is still on a valid output
        let still_valid = self.space.outputs_for_element(focused).iter().any(|o| o == output);
        if !still_valid {
            self.focused_window = None;
        }
    }
}


    pub fn poll_dbus(&mut self) {
        if let Some(service) = self.dbus_service.as_ref() {
            while let Ok((app_name, title, body)) = service.rx.try_recv() {
                self.notifications.push(&app_name, &title, &body);
                self.pending_full_redraw = true;
            }
        }
    }

    /// Tell mitos-session the user is still active, throttled so this
    /// fires on a timer rather than once per input event -- idle/lock
    /// timers just need a periodic "still here", not a continuous
    /// stream. Called from `input.rs`'s `process_input_event`, the one
    /// choke point every keyboard/pointer/gesture event already passes
    /// through.
    ///
    /// NOTE: `Request::ReportActivity`'s `seat_id` field type wasn't
    /// something I could check against mitos-session's own source from
    /// here (that crate isn't part of this project) -- this assumes a
    /// `u32`, matching the convention `session_id` already uses
    /// elsewhere in the same `Request` enum, and reuses this
    /// connection's own `session_id` as the seat id (reasonable for a
    /// single-seat desktop, which is the only case mitos-gui runs
    /// under today). Worth a quick check against
    /// mitos-session's `ipc/messages.rs` before trusting this compiles.
    pub fn report_activity(&mut self) {
        if self.last_activity_report.elapsed() < Duration::from_secs(15) {
            return;
        }
        self.last_activity_report = Instant::now();

        let Some(ipc) = self.session_ipc.as_mut() else { return };
        ipc.send(&mitos_session::ipc::Request::ReportActivity { seat_id: ipc.session_id.to_string() });
    }

    /// Drain events/replies from mitos-session and drive the lock
    /// screen accordingly. mitos-gui never decides on its own to lock
    /// or unlock anything -- it only ever reacts to what arrives here.
    pub fn poll_session_ipc(&mut self) {
        use mitos_session::ipc::{Event as SessionEvent, Message, Response};
        use mitos_session::authentication::AuthOutcome;
        use mitos_session::elevation::ElevationRisk;
        use mitos_session::lock::LockReason;

        let Some(ipc) = self.session_ipc.as_ref() else { return };

        let mut disconnected = false;

        loop {
            match ipc.rx.try_recv() {
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
                Ok(msg) => {
            match msg {
                Message::Event(SessionEvent::ShowLockScreen { reason, .. }) => {
                    let message = match reason {
                        LockReason::Manual => "Locked",
                        LockReason::Idle => "Locked (idle)",
                        LockReason::Suspend => "Locked (suspended)",
                    };
                    self.auth.lock(message);
                }
                Message::Event(SessionEvent::HideLockScreen { .. }) => {
                    self.auth.hide();
                }
                Message::Event(SessionEvent::AuthFeedback { outcome, .. }) => {
                    self.auth.pending = false;
                    match outcome {
                        AuthOutcome::Success => self.auth.error_msg = None,
                        AuthOutcome::Failure { attempts_remaining } => {
                            self.auth.error_msg = Some(format!("Incorrect password ({attempts_remaining} attempts left)"));
                        }
                        AuthOutcome::LockedOut { retry_after_secs } => {
                            self.auth.error_msg = Some(format!("Too many attempts -- try again in {retry_after_secs}s"));
                        }
                        AuthOutcome::Error(msg) => self.auth.error_msg = Some(msg),
                        // Unlock has no concept of "cancelled" -- this
                        // variant only ever comes from an elevation
                        // prompt (`ElevationFeedback`, below) -- but
                        // `AuthOutcome` is shared between the two flows,
                        // so it still has to be handled here.
                        AuthOutcome::Cancelled => {}
                    }
                }
                // mitos-session, relaying a request from mitos-service,
                // wants the logged-in user verified before a privileged
                // action proceeds -- see mitos-session's docs/security.md
                // for why only mitos-session (never the app itself) can
                // trigger this. `action` is display text only; this
                // module doesn't interpret it, same as `LockReason` above.
                Message::Event(SessionEvent::ShowElevationPrompt { request_id, action, .. }) => {
                    let critical = action.risk == ElevationRisk::Critical;
                    let risk = match action.risk {
                        ElevationRisk::Critical => "CRITICAL",
                        ElevationRisk::Elevated => "Elevated",
                    };
                    let subtitle = format!("{} · {risk} · {}", action.description, action.duration_label);
                    self.auth.request_elevation(request_id, &action.requesting_app, &subtitle, critical);
                }
                // One attempt against the prompt we're currently
                // showing was checked -- mirrors `AuthFeedback` above,
                // right down to never closing the prompt itself
                // (`HideElevationPrompt`, below, is the only thing that
                // does). Feedback for a request we're not currently
                // showing (already resolved locally, or still waiting
                // in the queue) is stale and ignored.
                Message::Event(SessionEvent::ElevationFeedback { request_id, outcome }) => {
                    if self.auth.request_id == Some(request_id) {
                        self.auth.pending = false;
                        self.auth.error_msg = match outcome {
                            AuthOutcome::Success => None,
                            AuthOutcome::Failure { attempts_remaining } => {
                                Some(format!("Incorrect password ({attempts_remaining} attempts left)"))
                            }
                            AuthOutcome::LockedOut { retry_after_secs } => {
                                Some(format!("Too many attempts -- try again in {retry_after_secs}s"))
                            }
                            AuthOutcome::Error(msg) => Some(msg),
                            // Someone else (root, via mitos-sessionctl)
                            // cancelled this prompt out from under us.
                            // `HideElevationPrompt` for the same request
                            // follows right behind and is what actually
                            // closes it, same as any other terminal
                            // outcome here.
                            AuthOutcome::Cancelled => Some("Cancelled".to_string()),
                        };
                    }
                }
                // The prompt for `request_id` is resolved, whatever the
                // reason (answered, cancelled, timed out, or its
                // session ended) -- dismiss it if it's the one we're
                // showing, or drop it from the queue if it was still
                // waiting its turn.
                Message::Event(SessionEvent::HideElevationPrompt { request_id }) => {
                    if self.auth.request_id == Some(request_id) {
                        self.auth.resolve_elevation();
                    } else {
                        self.auth.remove_from_queue(request_id);
                    }
                }
                Message::Response(resp @ Response::Session(_)) => {
                    // Reply to the startup `SessionStatus` query: pick
                    // up an already-locked session (e.g. mitos-gui
                    // restarted after a crash while locked).
                    if crate::session_ipc::session_locked(&resp) == Some(true) && !self.auth.active {
                        self.auth.lock("Locked");
                    }
                }
                    Message::Response(Response::Error(msg)) => {
                    // mitos-session refused something we asked for. The
                    // common case is LockSession rejected because the
                    // account has no password -- surface why, instead of
                    // the lock screen silently doing nothing.
                    let (title, body) =
                        if msg.contains("No password set") || msg.contains("Lock screen disabled")
                        {
                            (
                                "Cannot lock screen",
                                "No password is set for this user. Set one in Settings → Users → Change Password.",
                            )
                        } else {
                            ("Session manager error", msg.as_str())
                        };
                    self.notifications.push("MITOS Security", title, body);
                }
                _ => {}

            }
            self.pending_full_redraw = true;
                }
            }
        }

        if disconnected {
            // mitos-session's connection dropped (crash, restart, socket
            // error, ...). Whatever's on screen stays exactly as it is --
            // a lock screen fails closed, never open -- but a "checking
            // password..." spinner that can now never resolve (no more
            // `AuthFeedback` is ever coming) is its own kind of broken,
            // so say why instead of leaving Enter looking like it does
            // nothing. `self.session_ipc = None` also means Super+L,
            // `submit_lock_screen`, and `respond_elevation` all fall
            // into their existing "not connected to mitos-session"
            // paths from here on, rather than needing a second thing to
            // check.
            self.session_ipc = None;
            if self.auth.active {
                self.auth.pending = false;
                self.auth.error_msg = Some(
                    "Lost connection to mitos-session -- cannot verify password".to_string(),
                );
            }
            self.pending_full_redraw = true;
        }
    }
    

        /// Rebuilds the AT-SPI shell summary from current state and pushes
    /// it, syncs the notifications tree, announces launcher/auth state
    /// transitions, and drains any dock-item activations a screen
    /// reader triggered since the last call (see `atspi.rs`). Called
    /// once per frame from the main loop; cheap when nothing
    /// accessibility-relevant changed, which is most frames. No-ops
    /// entirely when no accessibility bus is connected.
    pub fn refresh_accessibility_summary(&mut self) {
        let mut parts = Vec::new();

        if self.shell.launcher_visible {
            if self.shell.launcher_query.is_empty() {
                parts.push("Launcher open".to_string());
            } else {
                parts.push(format!(
                    "Launcher: \"{}\", {} results",
                    self.shell.launcher_query,
                    self.shell.launcher_results.len(),
                ));
            }
        }

        if let Some(i) = self.shell.dock_focused {
            if let Some(item) = self.shell.dock_layout.items.get(i) {
                parts.push(format!("Dock: {} focused", item.name));
            }
        }

        if !self.notifications.active.is_empty() {
            parts.push(format!("{} notification(s)", self.notifications.active.len()));
        }

        if self.auth.active {
            parts.push(if self.auth.is_lock_screen {
                "Screen locked".to_string()
            } else {
                "Authentication requested".to_string()
            });
        }

        let description = if parts.is_empty() {
            "Idle".to_string()
        } else {
            parts.join("; ")
        };

        // Gathered up front so nothing below still needs to borrow
        // `self.shell`/`self.notifications`/`self.auth` while
        // `self.atspi` is borrowed mutably just after.
        let dock_focused = self.shell.dock_focused;
        let launcher_visible = self.shell.launcher_visible;
        let auth_active = self.auth.active;
        let active_notifs: Vec<(u32, String)> = self
            .notifications
            .active
            .iter()
            .map(|n| (n.id, n.title.clone()))
            .collect();
        let launcher_was = self.atspi_launcher_was_visible;
        let auth_was = self.atspi_auth_was_active;

        let pending_actions = if let Some(atspi) = self.atspi.as_mut() {
            atspi.update_summary(description, dock_focused);
            atspi.sync_notifications(&active_notifs);

            if launcher_visible != launcher_was {
                atspi.announce_state_change("showing", launcher_visible);
            }
            if auth_active != auth_was {
                atspi.announce_state_change("modal", auth_active);
            }

            atspi.poll_actions()
        } else {
            Vec::new()
        };

        self.atspi_launcher_was_visible = launcher_visible;
        self.atspi_auth_was_active = auth_active;

        for id in pending_actions {
            crate::shell_interaction::launch_app(self, &id);
        }
    }

    /// Switches the workspace on a specific monitor
    pub fn switch_workspace(&mut self, output_name: &str, ws: usize) {
        self.current_workspace.insert(output_name.to_string(), ws);
        self.pending_full_redraw = true;
    }

    /// Finds which monitor the pointer is currently hovering over
    pub fn active_output_name(&self) -> String {
        for output in &self.outputs {
            if let Some(geom) = self.space.output_geometry(output) {
                let ptr = self.pointer_location;
                if ptr.x >= geom.loc.x as f64 && ptr.x < (geom.loc.x + geom.size.w) as f64 &&
                   ptr.y >= geom.loc.y as f64 && ptr.y < (geom.loc.y + geom.size.h) as f64 {
                    return output.name();
                }
            }
        }
        self.outputs.first().map(|o| o.name()).unwrap_or_default()
    }


}

// ============================================================================
// SEAT HANDLER
// ============================================================================

impl SeatHandler for MitosGuiState {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(
        &mut self,
        _seat: &Seat<Self>,
        _focused: Option<&WlSurface>,
    ) {
        // Handled by wm::set_focus
    }

    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        _image: CursorImageStatus,
    ) {
        // MITOS cursor rendering will be implemented later.
    }
}
