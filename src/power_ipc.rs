//! Client for mitos-power's IPC (see `docs/ipc.md` in that repo):
//! newline-delimited JSON over `/run/mitos/power.sock`.
//!
//! mitos-power owns every power *decision* -- profiles, idle tiers, sleep,
//! battery policy, inhibitors. mitos-gui draws what it's told and does the one
//! thing the daemon can't: act on the display. This module only speaks the
//! wire protocol; `services.rs` turns the events it yields into UI state.
//!
//! The protocol types are deliberately *not* imported from the `mitos-power`
//! crate: it drags in tokio, udev and evdev, none of which a compositor should
//! link just to read a battery percentage. The wire format is small and
//! documented, so this is a thin re-implementation -- the same trade
//! mitos-power's own `session_client` makes for mitos-session.

use crate::ipc_line::{log_connect_failure, Backoff, LineConn};
use serde_json::{json, Value};

const DEFAULT_SOCKET_PATH: &str = "/run/mitos/power.sock";

/// Only the events the GUI actually reacts to. Everything else the daemon can
/// emit (lid, power button, inhibitor churn, suspend start, ...) never
/// crosses the socket, so the daemon doesn't even serialize it for us.
const SUBSCRIBED_EVENTS: [&str; 10] = [
    "BatteryChanged",
    "ChargingChanged",
    "ThermalWarning",
    "ThermalCritical",
    "ResumeFinished",
    "ProfileChanged",
    "BrightnessChanged",
    "IdleStateChanged",
    "ShutdownScheduled",
    "ShutdownCancelled",
];

/// Replies we're still waiting for. A daemon that never answers can't make
/// this grow: the oldest entry is dropped instead.
const MAX_PENDING: usize = 32;

/// Where the daemon says the display should be (`IdleStateChanged.tier`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdleTier {
    Awake,
    Dim,
    Off,
}

impl IdleTier {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "awake" => Some(Self::Awake),
            "dim" => Some(Self::Dim),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatteryReading {
    pub percent: u8,
    /// Charging or full (what the tray icon shows as "plugged in").
    pub charging: bool,
    /// Minutes to empty (discharging) or to full (charging), when known.
    pub eta_mins: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PowerSnapshot {
    pub battery: Option<BatteryReading>,
    /// The daemon's profile *name* -- the same strings `ListProfiles` returns
    /// and `SetProfile` accepts.
    pub profile: Option<String>,
    pub brightness: Option<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PowerEvent {
    Connected,
    Disconnected,
    Snapshot(PowerSnapshot),
    Profiles(Vec<String>),
    /// `None`: this machine has no (present) battery.
    Battery(Option<BatteryReading>),
    Thermal { critical: bool, temp_c: Option<f64> },
    /// The machine just woke from suspend/hibernate.
    Resumed,
    Profile(String),
    Brightness(u8),
    Idle(IdleTier),
    ShutdownScheduled { reboot: bool },
    ShutdownCancelled,
    /// Reply to `AcquireInhibitor`: the id to release later.
    InhibitorAcquired(String),
    /// A request the user asked for was refused (permission, inhibitor, ...).
    ActionFailed { what: &'static str, message: String },
}

/// What to do with the reply to a request we sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    Snapshot,
    Profiles,
    Inhibit,
    /// Fire-and-forget, but a *failure* is worth telling the user about.
    Action(&'static str),
}

pub struct PowerClient {
    path: String,
    conn: Option<LineConn>,
    backoff: Backoff,
    next_id: u64,
    pending: Vec<(u64, Pending)>,
    line: String,
    announced_down: bool,
    /// A send failed outside `poll`; report `Disconnected` on the next one.
    lost: bool,
}

impl PowerClient {
    pub fn new() -> Self {
        let path = std::env::var("MITOS_POWER_SOCKET")
            .unwrap_or_else(|_| DEFAULT_SOCKET_PATH.to_string());

        Self {
            path,
            conn: None,
            backoff: Backoff::new(),
            next_id: 1,
            pending: Vec::new(),
            line: String::new(),
            announced_down: false,
            lost: false,
        }
    }

    pub fn is_connected(&self) -> bool {
        self.conn.is_some()
    }

    /// Reads whatever the daemon has sent since the last call (and, while
    /// disconnected, occasionally tries to connect). Never blocks.
    pub fn poll(&mut self) -> Vec<PowerEvent> {
        let mut out = Vec::new();

        if self.lost {
            self.lost = false;
            out.push(PowerEvent::Disconnected);
        }

        if self.conn.is_none() {
            if self.backoff.due() {
                self.try_connect(&mut out);
            }
            return out;
        }

        let mut alive = true;
        if let Some(conn) = self.conn.as_mut() {
            if conn.has_pending_output() && !conn.flush() {
                alive = false;
            }
            if !conn.fill() {
                alive = false;
            }
        }

        // Drain complete lines even if the daemon just hung up: its last
        // words (a final battery update, say) may already be buffered.
        let mut line = std::mem::take(&mut self.line);
        loop {
            let got = match self.conn.as_mut() {
                Some(conn) => conn.next_line(&mut line),
                None => false,
            };
            if !got {
                break;
            }
            self.handle_line(&line, &mut out);
        }
        self.line = line;

        if !alive {
            self.drop_connection();
            out.push(PowerEvent::Disconnected);
        }

        out
    }

    // ------------------------------------------------------------------
    // Requests the GUI makes
    // ------------------------------------------------------------------

    /// "The user is here" -- resets mitos-power's idle clock. The daemon
    /// replies `{}`; we don't track it.
    pub fn report_activity(&mut self) -> bool {
        self.request("ReportActivity", Value::Null, None)
    }

    pub fn set_brightness(&mut self, percent: u8) -> bool {
        self.request(
            "SetBrightness",
            json!({ "percent": percent.min(100) }),
            Some(Pending::Action("Changing brightness")),
        )
    }

    pub fn set_profile(&mut self, name: &str) -> bool {
        self.request(
            "SetProfile",
            json!({ "name": name }),
            Some(Pending::Action("Switching power profile")),
        )
    }

    pub fn suspend(&mut self) -> bool {
        self.request("Suspend", Value::Null, Some(Pending::Action("Suspend")))
    }

    pub fn reboot(&mut self) -> bool {
        self.request("Reboot", Value::Null, Some(Pending::Action("Restart")))
    }

    pub fn poweroff(&mut self) -> bool {
        self.request("Poweroff", Value::Null, Some(Pending::Action("Shut down")))
    }

    /// Ask mitos-power not to dim, blank or idle-suspend while we hold this.
    /// The reply carries the id (`PowerEvent::InhibitorAcquired`); the daemon
    /// also drops it by itself if we disconnect.
    pub fn acquire_idle_inhibitor(&mut self, why: &str) -> bool {
        self.request(
            "AcquireInhibitor",
            json!({ "who": "mitos-gui", "why": why, "what": "idle" }),
            Some(Pending::Inhibit),
        )
    }

    pub fn release_inhibitor(&mut self, id: &str) -> bool {
        self.request("ReleaseInhibitor", json!({ "id": id }), None)
    }

    // ------------------------------------------------------------------
    // Connection handling
    // ------------------------------------------------------------------

    fn try_connect(&mut self, out: &mut Vec<PowerEvent>) {
        match LineConn::connect(&self.path) {
            Ok(conn) => {
                self.conn = Some(conn);
                self.backoff.succeeded();
                self.announced_down = false;
                tracing::info!("MITOS GUI: connected to mitos-power at {}", self.path);

                // Subscribe *before* asking for the snapshot, so no change
                // can slip in between the two.
                let subscribe = json!({ "kind": "subscribe", "events": SUBSCRIBED_EVENTS });
                self.send_line(subscribe.to_string());
                self.request("GetPowerState", Value::Null, Some(Pending::Snapshot));
                self.request("ListProfiles", Value::Null, Some(Pending::Profiles));

                if self.conn.is_some() {
                    out.push(PowerEvent::Connected);
                }
            }
            Err(err) => {
                log_connect_failure("mitos-power", &self.path, &err, &mut self.announced_down);
                self.backoff.failed();
            }
        }
    }

    fn drop_connection(&mut self) {
        if self.conn.take().is_some() {
            tracing::warn!("MITOS GUI: lost connection to mitos-power");
        }
        self.pending.clear();
        self.backoff.lost();
        self.announced_down = false;
    }

    fn send_line(&mut self, mut text: String) -> bool {
        text.push('\n');

        let ok = match self.conn.as_mut() {
            Some(conn) => conn.send(text.as_bytes()),
            None => return false,
        };

        if !ok {
            self.drop_connection();
            self.lost = true;
        }
        ok
    }

    fn request(&mut self, method: &str, params: Value, pending: Option<Pending>) -> bool {
        if self.conn.is_none() {
            return false;
        }

        let id = self.next_id;
        self.next_id += 1;

        let msg = json!({
            "kind": "request",
            "id": id.to_string(),
            "method": method,
            "params": params,
        });

        if !self.send_line(msg.to_string()) {
            return false;
        }

        if let Some(kind) = pending {
            if self.pending.len() >= MAX_PENDING {
                self.pending.remove(0);
            }
            self.pending.push((id, kind));
        }

        true
    }

    // ------------------------------------------------------------------
    // Incoming lines
    // ------------------------------------------------------------------

    fn handle_line(&mut self, line: &str, out: &mut Vec<PowerEvent>) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
        };

        match v.get("kind").and_then(Value::as_str) {
            Some("event") => parse_event(&v, out),
            Some("response") => self.handle_response(&v, out),
            _ => {}
        }
    }

    fn handle_response(&mut self, v: &Value, out: &mut Vec<PowerEvent>) {
        let Some(id) = v
            .get("id")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<u64>().ok())
        else {
            return;
        };

        let Some(pos) = self.pending.iter().position(|(pid, _)| *pid == id) else {
            return; // fire-and-forget, or from before a reconnect
        };
        let (_, kind) = self.pending.remove(pos);

        let ok = v.get("ok").and_then(Value::as_bool).unwrap_or(false);

        if !ok {
            let message = v
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("unknown error")
                .to_string();

            if let Pending::Action(what) = kind {
                out.push(PowerEvent::ActionFailed { what, message });
            }
            return;
        }

        let result = v.get("result").unwrap_or(&Value::Null);

        match kind {
            Pending::Snapshot => out.push(PowerEvent::Snapshot(parse_snapshot(result))),
            Pending::Profiles => {
                let names: Vec<String> = result
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .filter_map(|p| p.get("name").and_then(Value::as_str))
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                out.push(PowerEvent::Profiles(names));
            }
            Pending::Inhibit => {
                if let Some(inhibitor_id) = result.get("id").and_then(Value::as_str) {
                    out.push(PowerEvent::InhibitorAcquired(inhibitor_id.to_string()));
                }
            }
            Pending::Action(_) => {}
        }
    }
}

// ----------------------------------------------------------------------
// Pure parsing (unit-tested below)
// ----------------------------------------------------------------------

fn parse_event(v: &Value, out: &mut Vec<PowerEvent>) {
    let Some(name) = v.get("event").and_then(Value::as_str) else {
        return;
    };
    let data = v.get("data").unwrap_or(&Value::Null);

    match name {
        "BatteryChanged" | "ChargingChanged" => {
            out.push(PowerEvent::Battery(parse_battery_array(data)));
        }
        "ThermalWarning" | "ThermalCritical" => {
            out.push(PowerEvent::Thermal {
                critical: name == "ThermalCritical",
                temp_c: data.get("highest_temp_c").and_then(Value::as_f64),
            });
        }
        "ResumeFinished" => out.push(PowerEvent::Resumed),
        "ProfileChanged" => {
            if let Some(profile) = data.get("name").and_then(Value::as_str) {
                out.push(PowerEvent::Profile(profile.to_string()));
            }
        }
        "BrightnessChanged" => {
            if let Some(percent) = data.get("percent").and_then(Value::as_u64) {
                out.push(PowerEvent::Brightness(percent.min(100) as u8));
            }
        }
        "IdleStateChanged" => {
            if let Some(tier) = data
                .get("tier")
                .and_then(Value::as_str)
                .and_then(IdleTier::parse)
            {
                out.push(PowerEvent::Idle(tier));
            }
        }
        "ShutdownScheduled" => {
            out.push(PowerEvent::ShutdownScheduled {
                reboot: data.get("reboot").and_then(Value::as_bool).unwrap_or(false),
            });
        }
        "ShutdownCancelled" => out.push(PowerEvent::ShutdownCancelled),
        _ => {}
    }
}

/// The first *present* battery out of a `BatteryInfo[]`, boiled down to what
/// the shell shows. `None` when the machine has no battery.
fn parse_battery_array(v: &Value) -> Option<BatteryReading> {
    let list = v.as_array()?;

    let battery = list
        .iter()
        .find(|b| b.get("present").and_then(Value::as_bool).unwrap_or(true))?;

    let percent = battery
        .get("percentage")
        .and_then(Value::as_f64)?
        .round()
        .clamp(0.0, 100.0) as u8;

    let status = battery.get("status").and_then(Value::as_str).unwrap_or("");
    let full = status.eq_ignore_ascii_case("full");
    let charging = full || status.eq_ignore_ascii_case("charging");

    let eta_key = if charging {
        "time_to_full_secs"
    } else {
        "time_to_empty_secs"
    };

    let eta_mins = if full {
        None
    } else {
        battery
            .get(eta_key)
            .and_then(Value::as_u64)
            .map(|secs| ((secs + 30) / 60) as u32)
    };

    Some(BatteryReading {
        percent,
        charging,
        eta_mins,
    })
}

/// `GetPowerState` reports the profile as a `ProfileKind` (`{"kind": ...}`),
/// but `ListProfiles`/`SetProfile` use profile *names*. The built-in kinds
/// map to the names the daemon registers them under.
fn profile_name_from_kind(v: &Value) -> Option<String> {
    match v.get("kind").and_then(Value::as_str)? {
        "performance" => Some("performance".to_string()),
        "balanced" => Some("balanced".to_string()),
        "power_saver" => Some("powersave".to_string()),
        "custom" => v.get("name").and_then(Value::as_str).map(str::to_string),
        _ => None,
    }
}

fn parse_snapshot(result: &Value) -> PowerSnapshot {
    PowerSnapshot {
        battery: result.get("batteries").and_then(parse_battery_array),
        profile: result.get("profile").and_then(profile_name_from_kind),
        brightness: result
            .get("display_brightness_percent")
            .and_then(Value::as_u64)
            .map(|p| p.min(100) as u8),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(client: &mut PowerClient, text: &str) -> Vec<PowerEvent> {
        let mut out = Vec::new();
        client.handle_line(text, &mut out);
        out
    }

    #[test]
    fn battery_event_reports_percent_and_time_to_empty() {
        let mut c = PowerClient::new();
        let ev = feed(
            &mut c,
            r#"{"kind":"event","event":"BatteryChanged","data":[{"id":"BAT0","present":true,"percentage":76.3,"status":"discharging","time_to_empty_secs":15200,"time_to_full_secs":null}],"timestamp":"2026-09-24T14:03:11Z"}"#,
        );
        assert_eq!(
            ev,
            vec![PowerEvent::Battery(Some(BatteryReading {
                percent: 76,
                charging: false,
                eta_mins: Some(253),
            }))]
        );
    }

    #[test]
    fn charging_uses_time_to_full_and_full_has_no_eta() {
        let mut c = PowerClient::new();

        let ev = feed(
            &mut c,
            r#"{"kind":"event","event":"ChargingChanged","data":[{"present":true,"percentage":50.0,"status":"charging","time_to_empty_secs":null,"time_to_full_secs":1800}]}"#,
        );
        assert_eq!(
            ev,
            vec![PowerEvent::Battery(Some(BatteryReading {
                percent: 50,
                charging: true,
                eta_mins: Some(30),
            }))]
        );

        let ev = feed(
            &mut c,
            r#"{"kind":"event","event":"ChargingChanged","data":[{"present":true,"percentage":100.0,"status":"full","time_to_full_secs":0}]}"#,
        );
        assert_eq!(
            ev,
            vec![PowerEvent::Battery(Some(BatteryReading {
                percent: 100,
                charging: true,
                eta_mins: None,
            }))]
        );
    }

    #[test]
    fn no_present_battery_means_none() {
        let mut c = PowerClient::new();
        let ev = feed(
            &mut c,
            r#"{"kind":"event","event":"BatteryChanged","data":[{"present":false,"percentage":null}]}"#,
        );
        assert_eq!(ev, vec![PowerEvent::Battery(None)]);

        let ev = feed(&mut c, r#"{"kind":"event","event":"BatteryChanged","data":[]}"#);
        assert_eq!(ev, vec![PowerEvent::Battery(None)]);
    }

    #[test]
    fn simple_events() {
        let mut c = PowerClient::new();

        assert_eq!(
            feed(&mut c, r#"{"kind":"event","event":"BrightnessChanged","data":{"percent":140}}"#),
            vec![PowerEvent::Brightness(100)]
        );
        assert_eq!(
            feed(
                &mut c,
                r#"{"kind":"event","event":"IdleStateChanged","data":{"idle_seconds":300,"tier":"dim"}}"#
            ),
            vec![PowerEvent::Idle(IdleTier::Dim)]
        );
        assert_eq!(
            feed(
                &mut c,
                r#"{"kind":"event","event":"ProfileChanged","data":{"name":"powersave","kind":{"kind":"power_saver"},"is_custom":false}}"#
            ),
            vec![PowerEvent::Profile("powersave".to_string())]
        );
        assert_eq!(
            feed(&mut c, r#"{"kind":"event","event":"ResumeFinished","data":{}}"#),
            vec![PowerEvent::Resumed]
        );
        assert_eq!(
            feed(
                &mut c,
                r#"{"kind":"event","event":"ThermalCritical","data":{"level":"critical","highest_temp_c":96.5}}"#
            ),
            vec![PowerEvent::Thermal {
                critical: true,
                temp_c: Some(96.5),
            }]
        );
    }

    #[test]
    fn snapshot_reply_is_matched_by_id_and_maps_profile_kind_to_name() {
        let mut c = PowerClient::new();
        c.pending.push((7, Pending::Snapshot));

        let ev = feed(
            &mut c,
            r#"{"kind":"response","id":"7","ok":true,"result":{"on_ac":true,"batteries":[{"present":true,"percentage":50.0,"status":"charging","time_to_full_secs":1800}],"overall_percentage":50.0,"profile":{"kind":"power_saver"},"lid_closed":false,"display_brightness_percent":70,"thermal_level":"nominal","active_inhibitors":0,"idle_seconds":3}}"#,
        );

        assert_eq!(
            ev,
            vec![PowerEvent::Snapshot(PowerSnapshot {
                battery: Some(BatteryReading {
                    percent: 50,
                    charging: true,
                    eta_mins: Some(30),
                }),
                profile: Some("powersave".to_string()),
                brightness: Some(70),
            })]
        );
        assert!(c.pending.is_empty());
    }

    #[test]
    fn custom_profile_kind_uses_its_own_name() {
        let kind: Value = serde_json::from_str(r#"{"kind":"custom","name":"studio"}"#).unwrap();
        assert_eq!(profile_name_from_kind(&kind), Some("studio".to_string()));
    }

    #[test]
    fn profile_list_and_inhibitor_replies() {
        let mut c = PowerClient::new();
        c.pending.push((1, Pending::Profiles));
        c.pending.push((2, Pending::Inhibit));

        let ev = feed(
            &mut c,
            r#"{"kind":"response","id":"1","ok":true,"result":[{"name":"performance"},{"name":"balanced"},{"name":"powersave"},{"name":"studio"}]}"#,
        );
        assert_eq!(
            ev,
            vec![PowerEvent::Profiles(vec![
                "performance".to_string(),
                "balanced".to_string(),
                "powersave".to_string(),
                "studio".to_string(),
            ])]
        );

        let ev = feed(
            &mut c,
            r#"{"kind":"response","id":"2","ok":true,"result":{"id":"6f1c0c1e"}}"#,
        );
        assert_eq!(ev, vec![PowerEvent::InhibitorAcquired("6f1c0c1e".to_string())]);
    }

    #[test]
    fn failed_action_surfaces_the_daemons_message() {
        let mut c = PowerClient::new();
        c.pending.push((3, Pending::Action("Suspend")));

        let ev = feed(
            &mut c,
            r#"{"kind":"response","id":"3","ok":false,"error":{"code":"INHIBITED","message":"mitos-video: Movie is playing"}}"#,
        );
        assert_eq!(
            ev,
            vec![PowerEvent::ActionFailed {
                what: "Suspend",
                message: "mitos-video: Movie is playing".to_string(),
            }]
        );
    }

    #[test]
    fn unknown_ids_garbage_and_unsubscribed_events_are_ignored() {
        let mut c = PowerClient::new();

        assert!(feed(&mut c, r#"{"kind":"response","id":"99","ok":true,"result":{}}"#).is_empty());
        assert!(feed(&mut c, "not json at all").is_empty());
        assert!(feed(&mut c, r#"{"kind":"event","event":"LidChanged","data":{"closed":true}}"#).is_empty());
        // Not subscribed to (the battery's own charging state already says "plugged in").
        assert!(feed(&mut c, r#"{"kind":"event","event":"ACChanged","data":{"online":true}}"#).is_empty());
        assert!(feed(&mut c, r#"{"kind":"response","id":"abc","ok":true}"#).is_empty());
    }
}
