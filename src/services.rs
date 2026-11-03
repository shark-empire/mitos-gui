//! Glue between the shell and the three daemons it talks to:
//!
//! | daemon           | client            | what the shell does with it                              |
//! |------------------|-------------------|----------------------------------------------------------|
//! | mitos-power      | `power_ipc`       | battery/AC in the tray, brightness, profiles, idle dim/blank, sleep/shutdown, inhibitors |
//! | mitos-bluetooth  | `bluetooth_ipc`   | tray icon, Quick Settings toggle + device batteries, toasts, pairing prompt |
//! | mitos-input      | `input_ipc`       | device hot-plug toasts, on-screen keyboard policy, touchscreen gestures |
//!
//! The clients only speak wire protocols and yield events; everything that
//! touches shell state lives here. Every daemon is optional: with one
//! missing, its feature quietly falls back to what the shell did before
//! (sysfs battery polling, `brightnessctl`, signals to PID 1) or simply
//! doesn't appear.
//!
//! Cost: no threads, no channels, no polling of sysfs while mitos-power is
//! up -- see `ipc_line.rs`.

use std::time::{Duration, Instant};

use crate::bluetooth_ipc::{BluetoothClient, BtEvent, PairMethod};
use crate::context_menu::{ContextMenu, MenuAction};
use crate::input_ipc::{DeviceKind, InputClient, InputEvent, SwipeDir};
use crate::power_ipc::{BatteryReading, IdleTier, PowerClient, PowerEvent};
use crate::state::{MitosGuiState, OsdIcon, WORKSPACE_COUNT};
use crate::status::BatteryStatus;

/// Brightness step for the keyboard fallback (when mitos-power isn't there
/// to handle the key itself).
const BRIGHTNESS_STEP: u8 = 5;

/// mitos-power's evdev watcher normally handles the brightness keys itself;
/// if no `BrightnessChanged` follows a key press within this long, it isn't
/// doing so (disabled, or no device access) and the shell steps it instead.
const BRIGHTNESS_KEY_GRACE: Duration = Duration::from_millis(400);

/// A `BrightnessChanged` this soon after the shell set brightness itself is
/// the echo of that change, not news.
const OWN_BRIGHTNESS_ECHO: Duration = Duration::from_millis(400);

/// How often input is forwarded to mitos-power's idle clock while the screen
/// is awake (it is forwarded at once when waking from dim/blank).
const POWER_ACTIVITY_INTERVAL: Duration = Duration::from_secs(5);

const INHIBIT_CHECK_INTERVAL: Duration = Duration::from_secs(2);

/// Same failure toast at most this often (a slider drag against a machine
/// with no backlight would otherwise toast on every pixel).
const FAILURE_TOAST_INTERVAL: Duration = Duration::from_secs(5);

/// Bluetooth device battery warning level, re-armed 15 points above it.
const LOW_DEVICE_BATTERY: u8 = 15;

/// How much the display is darkened in the idle "dim" tier.
const DIM_ALPHA: f32 = 0.55;

/// Ascending (percent, title): one toast each time the charge drops to a
/// new level while discharging.
const BATTERY_WARNINGS: [(u8, &str); 3] = [
    (5, "Battery critically low"),
    (10, "Battery very low"),
    (20, "Battery low"),
];

#[derive(Clone, Debug)]
enum Inhibitor {
    None,
    /// `AcquireInhibitor` sent, id not back yet.
    Requested,
    Held(String),
}

pub struct Services {
    pub power: PowerClient,
    pub input: InputClient,
    pub bluetooth: BluetoothClient,

    // ---- mitos-power derived
    /// The daemon's name for the active profile (what `SetProfile` takes).
    profile: Option<String>,
    profiles: Vec<String>,
    /// Quick Settings button text, rebuilt only when it changes.
    profile_label: String,
    idle_tier: IdleTier,
    last_power_activity: Instant,
    last_own_brightness_set: Instant,
    /// A brightness key the shell handed to mitos-power: (when, up?).
    pending_brightness_key: Option<(Instant, bool)>,
    /// Lowest low-battery threshold already announced (100 = none).
    battery_warned: u8,
    inhibitor: Inhibitor,
    want_inhibit: bool,
    last_inhibit_check: Instant,
    last_failure: Option<(String, Instant)>,

    // ---- mitos-input derived
    auto_osk: bool,
    osk_auto_shown: bool,
    input_listed: bool,

    // ---- mitos-bluetooth derived
    /// Address of the pairing request the on-screen prompt is about.
    pending_pair: Option<String>,
    /// Devices already warned about a low battery (re-armed on recovery).
    low_battery_warned: Vec<String>,
}

impl Services {
    pub fn new() -> Self {
        let now = Instant::now();

        let mut services = Self {
            power: PowerClient::new(),
            input: InputClient::new(),
            bluetooth: BluetoothClient::new(),
            profile: None,
            profiles: Vec::new(),
            profile_label: String::new(),
            idle_tier: IdleTier::Awake,
            last_power_activity: now,
            last_own_brightness_set: now,
            pending_brightness_key: None,
            battery_warned: 100,
            inhibitor: Inhibitor::None,
            want_inhibit: false,
            last_inhibit_check: now,
            last_failure: None,
            auto_osk: std::env::var("MITOS_GUI_AUTO_OSK")
                .map(|v| v != "0")
                .unwrap_or(true),
            osk_auto_shown: false,
            input_listed: false,
            pending_pair: None,
            low_battery_warned: Vec::new(),
        };

        services.refresh_profile_label();
        services
    }

    /// Text of the Quick Settings power-profile button.
    pub fn profile_label(&self) -> &str {
        &self.profile_label
    }

    /// Whether the active profile is anything but the default (the button
    /// is drawn highlighted then, like an enabled toggle).
    pub fn profile_is_non_default(&self) -> bool {
        matches!(self.profile.as_deref(), Some(p) if p != "balanced")
    }

    fn refresh_profile_label(&mut self) {
        let label = if !self.power.is_connected() {
            "Power: n/a".to_string()
        } else {
            match self.profile.as_deref() {
                Some(p) => format!("Power: {}", pretty_profile(p)),
                None => "Power".to_string(),
            }
        };

        self.profile_label = label.chars().take(20).collect();
    }
}

// ============================================================================
// Per-frame entry point
// ============================================================================

impl MitosGuiState {
    /// Called once per frame by both backends, right after the other
    /// `poll_*` hooks. Never blocks; with nothing connected and nothing due
    /// it's a handful of clock reads.
    pub fn poll_services(&mut self) {
        let power_events = self.services.power.poll();
        for event in power_events {
            self.apply_power_event(event);
        }

        let input_events = self.services.input.poll();
        for event in input_events {
            self.apply_input_event(event);
        }

        let bluetooth_events = self.services.bluetooth.poll();
        for event in bluetooth_events {
            self.apply_bluetooth_event(event);
        }

        self.tick_services();
    }

    fn tick_services(&mut self) {
        // A brightness key the shell left to mitos-power that nobody answered.
        if let Some((since, up)) = self.services.pending_brightness_key {
            if since.elapsed() >= BRIGHTNESS_KEY_GRACE {
                self.services.pending_brightness_key = None;
                self.apply_brightness_step(up);
            }
        }

        // Keep the idle inhibitor in step with fullscreen windows.
        if self.services.power.is_connected()
            && self.services.last_inhibit_check.elapsed() >= INHIBIT_CHECK_INTERVAL
        {
            self.services.last_inhibit_check = Instant::now();
            self.sync_idle_inhibitor();
        }
    }

    // ------------------------------------------------------------------
    // What the renderer and input handlers ask for
    // ------------------------------------------------------------------

    /// 0.0 = normal, up to 1.0 = fully blanked: how much the whole screen is
    /// covered in black because mitos-power says the display is idle.
    pub fn screen_dim_alpha(&self) -> f32 {
        match self.services.idle_tier {
            IdleTier::Awake => 0.0,
            IdleTier::Dim => DIM_ALPHA,
            IdleTier::Off => 1.0,
        }
    }

    /// Tray Bluetooth icon: 0 = hidden, 1 = on, 2 = a device is connected.
    pub fn bluetooth_tray_state(&self) -> u8 {
        self.services.bluetooth.tray_state()
    }

    /// Input just happened. Tells mitos-power (which decides when to dim,
    /// blank or sleep) that someone is here.
    ///
    /// Returns true when this input woke a *blanked* screen: the caller
    /// should swallow the event, so the key press or click that lights the
    /// screen doesn't also act on whatever was under it, unseen.
    pub fn report_power_activity(&mut self) -> bool {
        if self.services.idle_tier != IdleTier::Awake {
            let was_off = self.services.idle_tier == IdleTier::Off;

            // Wake locally right away instead of waiting for the daemon's
            // IdleStateChanged round trip.
            self.services.idle_tier = IdleTier::Awake;
            self.services.last_power_activity = Instant::now();
            self.services.power.report_activity();
            self.pending_full_redraw = true;

            return was_off;
        }

        if self.services.last_power_activity.elapsed() >= POWER_ACTIVITY_INTERVAL {
            self.services.last_power_activity = Instant::now();
            self.services.power.report_activity();
        }

        false
    }

    /// Set the backlight: through mitos-power when it's up (it persists the
    /// level and tells every other client), else `brightnessctl`/`light`.
    pub fn set_brightness(&mut self, percent: u8) {
        let percent = percent.min(100);

        self.brightness = percent;
        self.services.last_own_brightness_set = Instant::now();

        if !self.services.power.set_brightness(percent) {
            crate::brightness::set_brightness(percent);
        }
    }

    /// A brightness key was pressed. With mitos-power up its own key watcher
    /// takes it (and the OSD follows its `BrightnessChanged`); the shell only
    /// steps the level itself if that doesn't happen in time. Stepping here
    /// *and* in the daemon would move the level twice per press.
    pub fn brightness_key(&mut self, up: bool) {
        if self.services.power.is_connected() {
            self.services.pending_brightness_key = Some((Instant::now(), up));
        } else {
            self.apply_brightness_step(up);
        }
    }

    fn apply_brightness_step(&mut self, up: bool) {
        let target = if up {
            self.brightness.saturating_add(BRIGHTNESS_STEP).min(100)
        } else {
            self.brightness.saturating_sub(BRIGHTNESS_STEP)
        };

        self.set_brightness(target);
        self.osd.trigger(OsdIcon::Brightness, target as f32 / 100.0);
        self.pending_full_redraw = true;
    }

    /// Restart. Through mitos-power (honours inhibitors, checks
    /// permissions); without it, the old signal to PID 1.
    pub fn request_reboot(&mut self) {
        if !self.services.power.reboot() {
            crate::session::reboot();
        }
    }

    pub fn request_poweroff(&mut self) {
        if !self.services.power.poweroff() {
            crate::session::poweroff();
        }
    }

    pub fn request_suspend(&mut self) {
        if !self.services.power.suspend() {
            self.notifications
                .push("Power", "Can't suspend", "mitos-power isn't running.");
        }
    }

    /// Quick Settings: next power profile, in the order the daemon lists them.
    pub fn cycle_power_profile(&mut self) {
        if !self.services.power.is_connected() || self.services.profiles.is_empty() {
            self.notifications.push(
                "Power",
                "Power profiles unavailable",
                "mitos-power isn't running.",
            );
            return;
        }

        let profiles = &self.services.profiles;
        let next = match self
            .services
            .profile
            .as_deref()
            .and_then(|current| profiles.iter().position(|p| p == current))
        {
            Some(i) => profiles[(i + 1) % profiles.len()].clone(),
            None => profiles[0].clone(),
        };

        self.services.power.set_profile(&next);
    }

    /// Quick Settings: Bluetooth on/off.
    pub fn toggle_bluetooth(&mut self) {
        if !self.services.bluetooth.is_connected() {
            self.notifications.push(
                "Bluetooth",
                "Bluetooth unavailable",
                "The Bluetooth service isn't running.",
            );
            return;
        }

        if !self.services.bluetooth.toggle_power() {
            self.notifications
                .push("Bluetooth", "Bluetooth unavailable", "No Bluetooth adapter found.");
        }
    }

    /// A button of the on-screen pairing prompt was clicked.
    pub fn respond_bluetooth_pairing(&mut self, accept: bool) {
        if let Some(address) = self.services.pending_pair.take() {
            self.services.bluetooth.pairing_confirm(&address, accept);
        }
    }

    // ------------------------------------------------------------------
    // mitos-power events
    // ------------------------------------------------------------------

    fn apply_power_event(&mut self, event: PowerEvent) {
        match event {
            PowerEvent::Connected => {}
            PowerEvent::Disconnected => {
                let s = &mut self.services;
                s.idle_tier = IdleTier::Awake; // never leave the screen black
                s.pending_brightness_key = None;
                s.inhibitor = Inhibitor::None; // the daemon drops ours with the connection
                s.profile = None;
                s.profiles.clear();
                s.refresh_profile_label();

                // Back to reading the battery from sysfs, starting now.
                self.last_status_poll = long_ago();
                self.pending_full_redraw = true;
            }
            PowerEvent::Snapshot(snapshot) => {
                self.battery = snapshot.battery.map(to_battery_status);
                if let Some(percent) = snapshot.brightness {
                    self.brightness = percent;
                }
                self.services.profile = snapshot.profile;
                self.services.refresh_profile_label();
                self.pending_full_redraw = true;
            }
            PowerEvent::Profiles(list) => {
                self.services.profiles = list;
            }
            PowerEvent::Battery(reading) => {
                self.battery = reading.map(to_battery_status);

                if let Some(r) = reading {
                    let title = battery_warning(r.percent, r.charging, &mut self.services.battery_warned);
                    if let Some(title) = title {
                        let body = format!("{}% remaining. Plug in soon.", r.percent);
                        self.notifications.push("Power", title, &body);
                    }
                }
                self.pending_full_redraw = true;
            }
            PowerEvent::Thermal { critical, temp_c } => {
                let temp = temp_c.map(|t| format!(" ({t:.0} C)")).unwrap_or_default();

                if critical {
                    let body = format!(
                        "Critical temperature{temp}. The system may shut down to protect itself."
                    );
                    self.notifications.push("Power", "System too hot", &body);
                } else {
                    let body = format!(
                        "Temperature is high{temp}. Close heavy applications to cool down."
                    );
                    self.notifications.push("Power", "System running hot", &body);
                }
            }
            PowerEvent::Resumed => {
                self.services.idle_tier = IdleTier::Awake;
                self.services.pending_brightness_key = None;

                // Network and battery may have changed while asleep.
                self.last_status_poll = long_ago();
                self.pending_full_redraw = true;
            }
            PowerEvent::Profile(name) => {
                self.services.profile = Some(name);
                self.services.refresh_profile_label();
            }
            PowerEvent::Brightness(percent) => {
                self.brightness = percent;
                self.services.pending_brightness_key = None;

                let ours = self.services.last_own_brightness_set.elapsed() < OWN_BRIGHTNESS_ECHO;
                if self.services.idle_tier == IdleTier::Awake && !ours {
                    self.osd
                        .trigger(OsdIcon::Brightness, percent as f32 / 100.0);
                }
                self.pending_full_redraw = true;
            }
            PowerEvent::Idle(tier) => {
                if self.services.idle_tier != tier {
                    self.services.idle_tier = tier;
                    self.pending_full_redraw = true;
                }
            }
            PowerEvent::ShutdownScheduled { reboot } => {
                let title = if reboot {
                    "Restart scheduled"
                } else {
                    "Shutdown scheduled"
                };
                self.notifications.push(
                    "Power",
                    title,
                    "The system will go down shortly. Save your work.",
                );
            }
            PowerEvent::ShutdownCancelled => {
                self.notifications.push(
                    "Power",
                    "Shutdown cancelled",
                    "The scheduled shutdown was cancelled.",
                );
            }
            PowerEvent::InhibitorAcquired(id) => {
                if self.services.want_inhibit {
                    self.services.inhibitor = Inhibitor::Held(id);
                } else {
                    // The window went away while the request was in flight.
                    self.services.power.release_inhibitor(&id);
                    self.services.inhibitor = Inhibitor::None;
                }
            }
            PowerEvent::ActionFailed { what, message } => {
                self.toast_failure("Power", what, &message);
            }
        }
    }

    /// Hold an idle inhibitor while any window is fullscreen, so mitos-power
    /// doesn't dim or blank the screen under a video. (A heuristic: apps
    /// that ask for it explicitly through `zwp_idle_inhibit_v1` would be
    /// the precise signal; this protocol isn't wired up yet.)
    fn sync_idle_inhibitor(&mut self) {
        let want = self
            .space
            .elements()
            .any(|window| crate::wm::meta(window).fullscreen);

        self.services.want_inhibit = want;

        match (self.services.inhibitor.clone(), want) {
            (Inhibitor::None, true) => {
                if self
                    .services
                    .power
                    .acquire_idle_inhibitor("A window is fullscreen")
                {
                    self.services.inhibitor = Inhibitor::Requested;
                }
            }
            (Inhibitor::Held(id), false) => {
                self.services.power.release_inhibitor(&id);
                self.services.inhibitor = Inhibitor::None;
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------
    // mitos-input events
    // ------------------------------------------------------------------

    fn apply_input_event(&mut self, event: InputEvent) {
        match event {
            InputEvent::Connected | InputEvent::Disconnected => {
                self.services.input_listed = false;
            }
            InputEvent::DeviceAdded {
                kind,
                name,
                initial,
            } => {
                if !initial {
                    if kind.is_notable() {
                        let body = if name.is_empty() { kind.label() } else { name.as_str() };
                        self.notifications.push(
                            "Input",
                            &format!("{} connected", kind.label()),
                            body,
                        );
                    }

                    // A real keyboard appeared: the on-screen one we raised
                    // for lack of it can go.
                    if kind == DeviceKind::Keyboard && self.services.osk_auto_shown {
                        self.osk_visible = false;
                        self.services.osk_auto_shown = false;
                        self.pending_full_redraw = true;
                    }
                }
            }
            InputEvent::DeviceRemoved { kind, name } => {
                if kind.is_notable() {
                    let body = if name.is_empty() { kind.label() } else { name.as_str() };
                    self.notifications.push(
                        "Input",
                        &format!("{} disconnected", kind.label()),
                        body,
                    );
                }
                if kind == DeviceKind::Keyboard {
                    self.update_osk_policy();
                }
            }
            InputEvent::Listed => {
                self.services.input_listed = true;
                self.update_osk_policy();
            }
            InputEvent::Swipe {
                from,
                fingers,
                dir,
            } => {
                // The lock/elevation prompt is modal, gestures included.
                if self.auth.active {
                    return;
                }

                let output = self.active_output_name();
                let current = self.current_workspace.get(&output).copied().unwrap_or(0);

                match swipe_action(
                    from,
                    fingers,
                    dir,
                    self.overview_output.is_some(),
                    current,
                ) {
                    Some(SwipeAction::OpenOverview) | Some(SwipeAction::CloseOverview) => {
                        self.toggle_overview();
                    }
                    Some(SwipeAction::Workspace(ws)) => self.switch_workspace(&output, ws),
                    None => {}
                }
            }
        }
    }

    /// No keyboard but a touchscreen: raise the on-screen keyboard (it's how
    /// you'd type the unlock password). Only touches it when it's off, and
    /// only takes it back down if it was this that put it up.
    fn update_osk_policy(&mut self) {
        if !self.services.auto_osk || !self.services.input_listed {
            return;
        }

        let touch_only = self.services.input.has(DeviceKind::Touchscreen)
            && !self.services.input.has(DeviceKind::Keyboard);

        if touch_only && !self.osk_visible {
            self.osk_visible = true;
            self.services.osk_auto_shown = true;
            self.pending_full_redraw = true;
        }
    }

    // ------------------------------------------------------------------
    // mitos-bluetooth events
    // ------------------------------------------------------------------

    fn apply_bluetooth_event(&mut self, event: BtEvent) {
        match event {
            // The tray icon follows adapter/device state.
            BtEvent::Connected | BtEvent::Disconnected => {
                self.pending_full_redraw = true;
            }
            BtEvent::DeviceConnected { name } => {
                self.notifications
                    .push("Bluetooth", "Device connected", &name);
                self.pending_full_redraw = true;
            }
            BtEvent::DeviceDisconnected { name } => {
                self.notifications
                    .push("Bluetooth", "Device disconnected", &name);
                self.services.low_battery_warned.retain(|n| *n != name);
                self.pending_full_redraw = true;
            }
            BtEvent::DeviceBattery { name, percent } => {
                if percent <= LOW_DEVICE_BATTERY {
                    let already = self.services.low_battery_warned.iter().any(|n| *n == name);
                    if !already {
                        if self.services.low_battery_warned.len() < 16 {
                            self.services.low_battery_warned.push(name.clone());
                        }
                        let body = format!("{name} is at {percent}%.");
                        self.notifications
                            .push("Bluetooth", "Device battery low", &body);
                    }
                } else if percent >= LOW_DEVICE_BATTERY + 15 {
                    self.services.low_battery_warned.retain(|n| *n != name);
                }
            }
            BtEvent::Pairing {
                address,
                name,
                method,
                passkey,
            } => self.on_pairing_request(address, name, method, passkey),
            BtEvent::PairingDone {
                name,
                success,
                error,
            } => {
                self.close_pairing_prompt();

                if success {
                    let body = format!("{name} is ready to use.");
                    self.notifications.push("Bluetooth", "Paired", &body);
                } else {
                    let reason = error.unwrap_or_else(|| "the device refused".to_string());
                    let body = format!("{name}: {reason}");
                    self.notifications
                        .push("Bluetooth", "Pairing failed", &body);
                }
            }
            BtEvent::ActionFailed { what, message } => {
                self.toast_failure("Bluetooth", what, &message);
            }
        }
    }

    fn on_pairing_request(
        &mut self,
        address: String,
        name: String,
        method: PairMethod,
        passkey: Option<u32>,
    ) {
        // A locked machine takes no new devices.
        if self.auth.active {
            self.services.bluetooth.pairing_confirm(&address, false);
            let body = format!("{name} tried to pair while the screen was locked.");
            self.notifications
                .push("Bluetooth", "Pairing rejected", &body);
            return;
        }

        match method {
            // Nothing to answer here: the code is typed on the other device.
            PairMethod::ShowCode => {
                let body = format!("Enter {:06} on the device.", passkey.unwrap_or(0));
                self.notifications
                    .push("Bluetooth", &format!("Pair with {name}"), &body);
                return;
            }
            PairMethod::EnterPin => {
                self.notifications.push(
                    "Bluetooth",
                    &format!("{name} wants to pair"),
                    "It needs a PIN, which this prompt can't take yet. Pair it from Settings.",
                );
                return;
            }
            PairMethod::Confirm | PairMethod::Compare => {}
        }

        let (accept_label, body) = match (method, passkey) {
            (PairMethod::Compare, Some(code)) => (
                format!("Codes match: {code:06}"),
                format!("Is {code:06} shown on {name}?"),
            ),
            _ => (
                "Pair".to_string(),
                format!("{name} wants to pair with this computer."),
            ),
        };

        self.notifications
            .push("Bluetooth", "Pairing request", &body);

        // The prompt is a context menu: modal, dismissed with a click away,
        // and already has hit-testing and rendering.
        let size = crate::wm::output_size(self);
        self.context_menu = Some(ContextMenu {
            x: size.w / 2 - 105,
            y: size.h / 3,
            items: vec![
                (accept_label, MenuAction::BluetoothPair { accept: true }),
                (
                    "Reject".to_string(),
                    MenuAction::BluetoothPair { accept: false },
                ),
            ],
        });
        self.services.pending_pair = Some(address);
        self.pending_full_redraw = true;
    }

    /// Takes the pairing prompt down if it's still showing (the pairing
    /// finished some other way, e.g. confirmed on the device itself).
    fn close_pairing_prompt(&mut self) {
        self.services.pending_pair = None;

        let is_pairing_menu = self
            .context_menu
            .as_ref()
            .map(|menu| {
                menu.items
                    .iter()
                    .any(|(_, action)| matches!(action, MenuAction::BluetoothPair { .. }))
            })
            .unwrap_or(false);

        if is_pairing_menu {
            self.context_menu = None;
            self.pending_full_redraw = true;
        }
    }

    // ------------------------------------------------------------------
    // Shared
    // ------------------------------------------------------------------

    fn toast_failure(&mut self, app: &str, what: &str, message: &str) {
        if let Some((last, at)) = &self.services.last_failure {
            if last == what && at.elapsed() < FAILURE_TOAST_INTERVAL {
                return;
            }
        }

        self.services.last_failure = Some((what.to_string(), Instant::now()));
        self.notifications
            .push(app, &format!("{what} failed"), message);
    }
}

// ============================================================================
// Pure helpers (unit-tested below)
// ============================================================================

fn to_battery_status(r: BatteryReading) -> BatteryStatus {
    BatteryStatus {
        capacity: r.percent,
        charging: r.charging,
        eta_mins: r.eta_mins,
    }
}

/// An `Instant` far enough back that a "due every N seconds" check fires on
/// the next frame (falls back to now if the clock is too young to go back).
fn long_ago() -> Instant {
    Instant::now()
        .checked_sub(Duration::from_secs(3600))
        .unwrap_or_else(Instant::now)
}

/// Edge-triggered low-battery warnings. Returns the title to announce when
/// this reading drops to a new, lower threshold; `warned` is the lowest one
/// already announced (100 = none). Charging re-arms everything.
fn battery_warning(percent: u8, charging: bool, warned: &mut u8) -> Option<&'static str> {
    if charging {
        *warned = 100;
        return None;
    }

    let &(level, title) = BATTERY_WARNINGS.iter().find(|entry| percent <= entry.0)?;

    if level < *warned {
        *warned = level;
        Some(title)
    } else {
        None
    }
}

/// Display text for a daemon profile name.
fn pretty_profile(name: &str) -> String {
    if name == "powersave" {
        return "Power Saver".to_string();
    }

    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum SwipeAction {
    OpenOverview,
    CloseOverview,
    Workspace(usize),
}

/// What a touchscreen swipe does. Same convention as the touchpad swipe in
/// `gestures.rs` (fingers moving right bring the previous workspace in);
/// touchpads are left to libinput's own gestures, so only touchscreen swipes
/// from mitos-input count -- otherwise one touchpad swipe would act twice.
fn swipe_action(
    from: DeviceKind,
    fingers: u32,
    dir: SwipeDir,
    overview_open: bool,
    current_workspace: usize,
) -> Option<SwipeAction> {
    if from != DeviceKind::Touchscreen || fingers < 3 {
        return None;
    }

    match dir {
        SwipeDir::Up => (!overview_open).then_some(SwipeAction::OpenOverview),
        SwipeDir::Down => overview_open.then_some(SwipeAction::CloseOverview),
        SwipeDir::Right => current_workspace.checked_sub(1).map(SwipeAction::Workspace),
        SwipeDir::Left => (current_workspace + 1 < WORKSPACE_COUNT)
            .then_some(SwipeAction::Workspace(current_workspace + 1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_warnings_fire_once_per_threshold() {
        let mut warned = 100;

        assert_eq!(battery_warning(50, false, &mut warned), None);
        assert_eq!(battery_warning(20, false, &mut warned), Some("Battery low"));
        assert_eq!(battery_warning(19, false, &mut warned), None);
        assert_eq!(battery_warning(10, false, &mut warned), Some("Battery very low"));
        assert_eq!(
            battery_warning(4, false, &mut warned),
            Some("Battery critically low")
        );
        assert_eq!(battery_warning(3, false, &mut warned), None);

        // Plugging in re-arms every threshold.
        assert_eq!(battery_warning(3, true, &mut warned), None);
        assert_eq!(battery_warning(15, false, &mut warned), Some("Battery low"));
    }

    #[test]
    fn starting_very_low_gives_one_critical_toast_not_three() {
        let mut warned = 100;

        assert_eq!(
            battery_warning(4, false, &mut warned),
            Some("Battery critically low")
        );
        assert_eq!(battery_warning(4, false, &mut warned), None);
        assert_eq!(battery_warning(3, false, &mut warned), None);
    }

    #[test]
    fn profile_names_are_prettified() {
        assert_eq!(pretty_profile("powersave"), "Power Saver");
        assert_eq!(pretty_profile("balanced"), "Balanced");
        assert_eq!(pretty_profile("performance"), "Performance");
        assert_eq!(pretty_profile("studio"), "Studio");
        assert_eq!(pretty_profile(""), "");
    }

    #[test]
    fn touchscreen_swipes_map_to_overview_and_workspaces() {
        let ts = DeviceKind::Touchscreen;

        assert_eq!(
            swipe_action(ts, 3, SwipeDir::Up, false, 0),
            Some(SwipeAction::OpenOverview)
        );
        assert_eq!(swipe_action(ts, 3, SwipeDir::Up, true, 0), None);
        assert_eq!(
            swipe_action(ts, 3, SwipeDir::Down, true, 0),
            Some(SwipeAction::CloseOverview)
        );
        assert_eq!(swipe_action(ts, 3, SwipeDir::Down, false, 0), None);

        // Fingers moving left bring the next workspace in; right, the previous.
        assert_eq!(
            swipe_action(ts, 4, SwipeDir::Left, false, 1),
            Some(SwipeAction::Workspace(2))
        );
        assert_eq!(
            swipe_action(ts, 4, SwipeDir::Right, false, 1),
            Some(SwipeAction::Workspace(0))
        );

        // No wrapping at either end.
        assert_eq!(swipe_action(ts, 4, SwipeDir::Right, false, 0), None);
        assert_eq!(
            swipe_action(ts, 4, SwipeDir::Left, false, WORKSPACE_COUNT - 1),
            None
        );
    }

    #[test]
    fn only_multi_finger_touchscreen_swipes_count() {
        assert_eq!(swipe_action(DeviceKind::Touchscreen, 2, SwipeDir::Left, false, 0), None);
        // Touchpad swipes already reach the compositor through libinput.
        assert_eq!(swipe_action(DeviceKind::Touchpad, 3, SwipeDir::Left, false, 0), None);
        assert_eq!(swipe_action(DeviceKind::Unknown, 4, SwipeDir::Up, false, 0), None);
    }
}
