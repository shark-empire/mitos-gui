//! Client for mitos-audio's control plane (see `docs/ipc.md` in that repo):
//! newline-delimited JSON over `/run/mitos/audio.sock`.
//!
//! What the shell gets from it: the real master volume and mute state (tray
//! icon, Quick Settings, OSD -- including changes made by anything else,
//! e.g. `mitos-audioctl` or a settings app), the volume keys and Quick
//! Settings controls acting on the daemon instead of spawning `wpctl` /
//! `pactl` for every step, an output-device switcher, and the microphone
//! mute key.
//!
//! mitos-audio ships a typed client crate (`libmitos-audio`) and recommends
//! it for the GUI. It is built on tokio, which a compositor shouldn't link
//! (and run a runtime for) just to move a volume slider, so this is a thin
//! client over the same non-blocking transport as the other daemon clients:
//! no threads, no runtime, read once per frame. The wire format is small and
//! additive-only, so that is a fair trade.
//!
//! One connection does everything: `SubscribeEvents` is sent first and
//! `GetState` after it, so no change can slip between the snapshot and the
//! event stream; replies and events share the line stream and are told apart
//! by `"id"` vs `"event"`.
//!
//! Access: the socket is `0660` and the daemon only accepts root, its own
//! uid, or members of the `mitos-audio` group, so mitos-gui must run as one
//! of those.

use crate::ipc_line::{log_connect_failure, Backoff, LineConn};
use crate::wake::WakeSet;
use serde_json::{json, Value};

const DEFAULT_SOCKET_PATH: &str = "/run/mitos/audio.sock";

/// Replies we're still waiting for; the oldest is dropped past this.
const MAX_PENDING: usize = 32;

/// Output devices tracked. More than this is a broken daemon, not a machine.
const MAX_OUTPUTS: usize = 32;

/// Longest Quick Settings output-button text (it has to fit the button).
const MAX_LABEL_CHARS: usize = 28;

/// An output device the user can switch to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioOutput {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AudioEvent {
    Connected,
    Disconnected,
    /// Master volume and mute together: the first sync after connecting, or
    /// after the default output changed (each output has its own volume).
    Master { volume: u8, muted: bool },
    /// Master volume changed.
    Volume(u8),
    /// Master mute changed.
    Muted(bool),
    /// The default output changed, to the output with this display name.
    OutputChanged { name: String },
    /// The microphone was muted or unmuted.
    MicMuted(bool),
    /// Something the user asked for was refused.
    ActionFailed { what: &'static str, message: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    State,
    Devices,
    Volume,
    /// Fire-and-forget, but a failure is worth telling the user about.
    Action(&'static str),
}

pub struct AudioClient {
    path: String,
    conn: Option<LineConn>,
    backoff: Backoff,
    next_id: u64,
    pending: Vec<(u64, Pending)>,
    line: String,
    announced_down: bool,
    /// A send failed outside `poll`; report `Disconnected` on the next one.
    lost: bool,
    /// Lets the main loop sleep until the daemon says something.
    wake: Option<WakeSet>,

    /// Output (or duplex) devices that are present, in the daemon's order.
    outputs: Vec<AudioOutput>,
    default_output: Option<String>,
    mic_muted: Option<bool>,
    /// Quick Settings button text, rebuilt only when it changes (it is
    /// read every frame while the panel is open).
    label: String,
}

impl AudioClient {
    pub fn new() -> Self {
        let path = std::env::var("MITOS_AUDIO_SOCKET")
            .unwrap_or_else(|_| DEFAULT_SOCKET_PATH.to_string());

        let mut client = Self {
            path,
            conn: None,
            backoff: Backoff::new(),
            next_id: 1,
            pending: Vec::new(),
            line: String::new(),
            announced_down: false,
            lost: false,
            wake: None,
            outputs: Vec::new(),
            default_output: None,
            mic_muted: None,
            label: String::new(),
        };
        client.rebuild_label();
        client
    }

    /// Have the main loop wake when this daemon's socket has data
    /// (see `wake.rs`); without it the loop must poll.
    pub fn set_wake(&mut self, wake: WakeSet) {
        self.wake = Some(wake);
    }

    pub fn is_connected(&self) -> bool {
        self.conn.is_some()
    }

    /// Quick Settings output button text ("Output: Speakers").
    pub fn output_label(&self) -> &str {
        &self.label
    }

    /// `None` until the daemon has said (or while it isn't there).
    pub fn mic_muted(&self) -> Option<bool> {
        self.mic_muted
    }

    /// Reads whatever the daemon has sent since the last call (and, while
    /// disconnected, occasionally tries to connect). Never blocks.
    pub fn poll(&mut self) -> Vec<AudioEvent> {
        let mut out = Vec::new();

        if self.lost {
            self.lost = false;
            out.push(AudioEvent::Disconnected);
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
            out.push(AudioEvent::Disconnected);
        }

        out
    }

    // ------------------------------------------------------------------
    // Requests the GUI makes
    // ------------------------------------------------------------------

    /// Master volume, 0-100 (the daemon's `max_volume` defaults to 100).
    pub fn set_volume(&mut self, percent: u8) -> bool {
        self.request(
            "SetVolume",
            json!({ "volume": percent.min(100) }),
            Some(Pending::Action("Setting volume")),
        )
    }

    pub fn set_muted(&mut self, muted: bool) -> bool {
        let command = if muted { "Mute" } else { "Unmute" };
        self.request(command, json!({}), Some(Pending::Action("Changing mute")))
    }

    pub fn set_mic_muted(&mut self, mute: bool) -> bool {
        self.request(
            "MuteMicrophone",
            json!({ "mute": mute }),
            Some(Pending::Action("Muting the microphone")),
        )
    }

    /// Switch the default output to the next present output device. Returns
    /// the new output's name, or `None` if there is nothing to switch to
    /// (fewer than two outputs, or no daemon).
    pub fn cycle_output(&mut self) -> Option<String> {
        if self.conn.is_none() || self.outputs.len() < 2 {
            return None;
        }

        let current = self
            .default_output
            .as_deref()
            .and_then(|id| self.outputs.iter().position(|o| o.id == id));

        let next = match current {
            Some(i) => (i + 1) % self.outputs.len(),
            None => 0,
        };

        let target = self.outputs[next].clone();

        let sent = self.request(
            "SetDefaultOutput",
            json!({ "id": target.id }),
            Some(Pending::Action("Switching output")),
        );

        if sent {
            Some(target.name)
        } else {
            None
        }
    }

    // ------------------------------------------------------------------
    // Connection handling
    // ------------------------------------------------------------------

    fn try_connect(&mut self, out: &mut Vec<AudioEvent>) {
        match LineConn::connect(&self.path) {
            Ok(conn) => {
                self.conn = Some(conn);
                if let (Some(wake), Some(conn)) = (&self.wake, &self.conn) {
                    wake.add(conn.raw_fd());
                }
                self.backoff.succeeded();
                self.announced_down = false;
                tracing::info!("MITOS GUI: connected to mitos-audio at {}", self.path);

                // Subscribe *before* asking for the snapshot, so no change
                // can slip in between the two.
                self.request("SubscribeEvents", json!({}), None);
                self.request("GetState", json!({}), Some(Pending::State));

                if self.conn.is_some() {
                    self.rebuild_label();
                    out.push(AudioEvent::Connected);
                }
            }
            Err(err) => {
                log_connect_failure("mitos-audio", &self.path, &err, &mut self.announced_down);
                self.backoff.failed();
            }
        }
    }

    fn drop_connection(&mut self) {
        if self.conn.take().is_some() {
            tracing::warn!("MITOS GUI: lost connection to mitos-audio");
        }
        self.pending.clear();
        self.outputs.clear();
        self.default_output = None;
        self.mic_muted = None;
        self.backoff.lost();
        self.announced_down = false;
        self.rebuild_label();
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

    fn request(&mut self, command: &str, params: Value, pending: Option<Pending>) -> bool {
        if self.conn.is_none() {
            return false;
        }

        let id = self.next_id;
        self.next_id += 1;

        let msg = json!({ "id": id, "command": command, "params": params });

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

    /// Re-reads the device list (and defaults), unless that's already on
    /// its way: a hot-plug can fire several events at once.
    fn refresh_devices(&mut self) {
        if self.pending.iter().any(|(_, kind)| *kind == Pending::Devices) {
            return;
        }
        self.request("ListDevices", json!({}), Some(Pending::Devices));
    }

    // ------------------------------------------------------------------
    // Incoming lines
    // ------------------------------------------------------------------

    fn handle_line(&mut self, line: &str, out: &mut Vec<AudioEvent>) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
        };

        if let Some(name) = v.get("event").and_then(Value::as_str) {
            let data = v.get("data").unwrap_or(&Value::Null);
            self.handle_event(name, data, out);
        } else if v.get("id").is_some() {
            self.handle_response(&v, out);
        }
    }

    fn handle_event(&mut self, name: &str, data: &Value, out: &mut Vec<AudioEvent>) {
        match name {
            "VolumeChanged" => {
                // The daemon names the device whose volume changed; only the
                // default output's is "the volume" here.
                let device = data.get("device").and_then(Value::as_str);
                let is_master = match (device, self.default_output.as_deref()) {
                    (None, _) => true,
                    (Some(changed), Some(default)) => changed == default,
                    (Some(_), None) => false,
                };

                if is_master {
                    if let Some(volume) = data.get("volume").and_then(Value::as_u64) {
                        out.push(AudioEvent::Volume(volume.min(100) as u8));
                    }
                }
            }
            "MuteChanged" => {
                if let Some(muted) = data.get("muted").and_then(Value::as_bool) {
                    out.push(AudioEvent::Muted(muted));
                }
            }
            "DefaultOutputChanged" => {
                if let Some(id) = data.get("id").and_then(Value::as_str) {
                    self.default_output = Some(id.to_string());
                    self.rebuild_label();

                    // Every output has its own volume and mute.
                    self.request("GetVolume", json!({}), Some(Pending::Volume));

                    out.push(AudioEvent::OutputChanged {
                        name: self.output_name(id),
                    });
                }
            }
            "DeviceAdded" | "DeviceRemoved" | "DeviceChanged" => self.refresh_devices(),
            "MicrophoneChanged" => {
                // Every microphone change (gain, noise suppression, ...)
                // re-sends the whole microphone snapshot; only an actual
                // mute change is news.
                if let Some(muted) = data.get("muted").and_then(Value::as_bool) {
                    let changed = self.mic_muted != Some(muted);
                    self.mic_muted = Some(muted);

                    if changed {
                        out.push(AudioEvent::MicMuted(muted));
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_response(&mut self, v: &Value, out: &mut Vec<AudioEvent>) {
        let Some(id) = v.get("id").and_then(Value::as_u64) else {
            return;
        };

        let Some(pos) = self.pending.iter().position(|(pid, _)| *pid == id) else {
            return; // the subscribe ack, or from before a reconnect
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
                out.push(AudioEvent::ActionFailed { what, message });
            }
            return;
        }

        let result = v.get("result").unwrap_or(&Value::Null);

        match kind {
            Pending::State => {
                self.apply_devices(result);

                if let Some(muted) = result
                    .get("microphone")
                    .and_then(|m| m.get("muted"))
                    .and_then(Value::as_bool)
                {
                    self.mic_muted = Some(muted);
                }

                // With no default output the snapshot's volume is a
                // placeholder 0, which must not overwrite what's shown.
                if self.default_output.is_some() {
                    let volume = result
                        .get("master_volume")
                        .and_then(Value::as_u64)
                        .unwrap_or(0)
                        .min(100) as u8;
                    let muted = result
                        .get("master_muted")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);

                    out.push(AudioEvent::Master { volume, muted });
                }
            }
            Pending::Devices => self.apply_devices(result),
            Pending::Volume => {
                if let Some(volume) = result.get("volume").and_then(Value::as_u64) {
                    let muted = result.get("muted").and_then(Value::as_bool).unwrap_or(false);
                    out.push(AudioEvent::Master {
                        volume: volume.min(100) as u8,
                        muted,
                    });
                }
            }
            Pending::Action(_) => {}
        }
    }

    // ------------------------------------------------------------------
    // Device table
    // ------------------------------------------------------------------

    /// Takes the device list and default output from a `GetState` or
    /// `ListDevices` result.
    fn apply_devices(&mut self, result: &Value) {
        if let Some(list) = result.get("devices").and_then(Value::as_array) {
            self.outputs = parse_outputs(list);
        }

        self.default_output = result
            .get("default_output")
            .and_then(Value::as_str)
            .map(str::to_string);

        self.rebuild_label();
    }

    fn output_name(&self, id: &str) -> String {
        self.outputs
            .iter()
            .find(|o| o.id == id)
            .map(|o| o.name.clone())
            .unwrap_or_else(|| id.to_string())
    }

    fn rebuild_label(&mut self) {
        let text = if self.conn.is_none() {
            "Output: n/a".to_string()
        } else {
            match self.default_output.as_deref() {
                Some(id) => format!("Output: {}", self.output_name(id)),
                None => "Output: none".to_string(),
            }
        };

        self.label = clip(&text, MAX_LABEL_CHARS);
    }
}

// ----------------------------------------------------------------------
// Parsing (unit-tested below)
// ----------------------------------------------------------------------

/// Output (or duplex) devices that are actually there: an unplugged
/// headphone jack shouldn't be a stop on the output switcher.
fn parse_outputs(devices: &[Value]) -> Vec<AudioOutput> {
    let mut outputs = Vec::new();

    for device in devices {
        let Some(id) = device.get("id").and_then(Value::as_str) else {
            continue;
        };

        if device.get("direction").and_then(Value::as_str) == Some("input") {
            continue;
        }

        let state = device.get("state").and_then(Value::as_str).unwrap_or("available");
        if matches!(state, "disconnected" | "unavailable" | "error") {
            continue;
        }

        let name = device
            .get("name")
            .and_then(Value::as_str)
            .map(clean_name)
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| id.to_string());

        if outputs.len() < MAX_OUTPUTS {
            outputs.push(AudioOutput {
                id: id.to_string(),
                name,
            });
        }
    }

    outputs
}

/// Device names come from hardware (ALSA, Bluetooth): strip anything that
/// isn't plain printable text before it reaches the screen.
fn clean_name(raw: &str) -> String {
    raw.chars().filter(|c| !c.is_control()).take(40).collect()
}

fn clip(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    fn feed(c: &mut AudioClient, text: &str) -> Vec<AudioEvent> {
        let mut out = Vec::new();
        c.handle_line(text, &mut out);
        out
    }

    /// A client wired to one end of a socketpair.
    fn connected_client() -> (AudioClient, UnixStream) {
        let (a, b) = UnixStream::pair().unwrap();
        b.set_read_timeout(Some(Duration::from_secs(2))).unwrap();

        let mut c = AudioClient::new();
        c.conn = Some(LineConn::from_stream(a).unwrap());
        c.rebuild_label();
        (c, b)
    }

    const STATE: &str = r#"{"id":1,"ok":true,"result":{"service":"mitos-audio","version":"0.5.0","backend":"alsa","devices":[{"id":"speakers","name":"Built-in Speakers","kind":"speakers","direction":"output","state":"active","volume":42,"muted":false},{"id":"headphones","name":"Headphones","kind":"headphones","direction":"output","state":"available","volume":30,"muted":false},{"id":"unplugged","name":"Old USB DAC","kind":"usb","direction":"output","state":"disconnected","volume":50,"muted":false},{"id":"mic","name":"Built-in Mic","kind":"microphone","direction":"input","state":"active","volume":50,"muted":false}],"streams":[],"groups":[],"default_output":"speakers","default_input":"mic","master_volume":42,"master_muted":false,"microphone":{"muted":true,"gain_db":0,"noise_suppression":false,"echo_cancellation":false,"agc":false},"profile":"stereo"}}"#;

    fn synced_client() -> (AudioClient, UnixStream) {
        let (mut c, peer) = connected_client();
        c.pending.push((1, Pending::State));
        feed(&mut c, STATE);
        (c, peer)
    }

    #[test]
    fn the_first_snapshot_syncs_volume_outputs_and_mic() {
        let (mut c, peer) = connected_client();
        c.pending.push((1, Pending::State));

        let ev = feed(&mut c, STATE);
        assert_eq!(
            ev,
            vec![AudioEvent::Master {
                volume: 42,
                muted: false
            }]
        );
        assert_eq!(c.mic_muted(), Some(true));
        assert_eq!(c.output_label(), "Output: Built-in Speakers");

        // Inputs and unplugged devices are not stops on the switcher.
        let ids: Vec<&str> = c.outputs.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, vec!["speakers", "headphones"]);
        drop(peer);
    }

    #[test]
    fn a_snapshot_without_a_default_output_does_not_zero_the_volume() {
        let (mut c, _peer) = connected_client();
        c.pending.push((1, Pending::State));

        let ev = feed(
            &mut c,
            r#"{"id":1,"ok":true,"result":{"devices":[],"default_output":null,"master_volume":0,"master_muted":false}}"#,
        );
        assert!(ev.is_empty());
        assert_eq!(c.output_label(), "Output: none");
    }

    #[test]
    fn only_the_default_outputs_volume_is_the_volume() {
        let (mut c, _peer) = synced_client();

        assert_eq!(
            feed(
                &mut c,
                r#"{"event":"VolumeChanged","data":{"device":"speakers","volume":60}}"#
            ),
            vec![AudioEvent::Volume(60)]
        );
        assert_eq!(
            feed(
                &mut c,
                r#"{"event":"VolumeChanged","data":{"device":null,"volume":55}}"#
            ),
            vec![AudioEvent::Volume(55)]
        );
        assert!(feed(
            &mut c,
            r#"{"event":"VolumeChanged","data":{"device":"headphones","volume":10}}"#
        )
        .is_empty());

        // Over-range values are shown as 100, not wrapped.
        assert_eq!(
            feed(
                &mut c,
                r#"{"event":"VolumeChanged","data":{"device":"speakers","volume":4000}}"#
            ),
            vec![AudioEvent::Volume(100)]
        );
    }

    #[test]
    fn mute_and_mic_events() {
        let (mut c, _peer) = synced_client();

        assert_eq!(
            feed(&mut c, r#"{"event":"MuteChanged","data":{"muted":true}}"#),
            vec![AudioEvent::Muted(true)]
        );
        assert_eq!(
            feed(
                &mut c,
                r#"{"event":"MicrophoneChanged","data":{"muted":false,"gain":0,"noise_suppression":false,"echo_cancellation":false,"agc":false}}"#
            ),
            vec![AudioEvent::MicMuted(false)]
        );
        assert_eq!(c.mic_muted(), Some(false));

        // A gain change re-sends the whole snapshot with the mute state
        // unchanged: not news.
        assert!(feed(
            &mut c,
            r#"{"event":"MicrophoneChanged","data":{"muted":false,"gain":6,"noise_suppression":true,"echo_cancellation":false,"agc":false}}"#
        )
        .is_empty());
    }

    #[test]
    fn switching_the_default_output_resyncs_volume_and_names_the_output() {
        let (mut c, mut peer) = synced_client();

        let ev = feed(
            &mut c,
            r#"{"event":"DefaultOutputChanged","data":{"id":"headphones"}}"#,
        );
        assert_eq!(
            ev,
            vec![AudioEvent::OutputChanged {
                name: "Headphones".to_string()
            }]
        );
        assert_eq!(c.output_label(), "Output: Headphones");

        // It asked for the new output's own volume ...
        let mut buf = [0u8; 256];
        let n = peer.read(&mut buf).unwrap();
        assert!(String::from_utf8_lossy(&buf[..n]).contains(r#""command":"GetVolume""#));

        // ... and applies the answer.
        let id = c.pending.last().unwrap().0;
        let reply = r#"{"id":@ID@,"ok":true,"result":{"device":"headphones","volume":30,"muted":true}}"#
            .replace("@ID@", &id.to_string());
        assert_eq!(
            feed(&mut c, &reply),
            vec![AudioEvent::Master {
                volume: 30,
                muted: true
            }]
        );
    }

    #[test]
    fn device_events_ask_for_one_refresh_not_one_per_event() {
        let (mut c, mut peer) = synced_client();

        feed(&mut c, r#"{"event":"DeviceAdded","data":{"id":"x","name":"X"}}"#);
        feed(&mut c, r#"{"event":"DeviceChanged","data":{"id":"x"}}"#);
        feed(&mut c, r#"{"event":"DeviceRemoved","data":{"id":"y"}}"#);

        let mut buf = [0u8; 512];
        let n = peer.read(&mut buf).unwrap();
        let sent = String::from_utf8_lossy(&buf[..n]).to_string();
        assert_eq!(sent.matches(r#""command":"ListDevices""#).count(), 1);
    }

    #[test]
    fn the_device_list_reply_updates_the_switcher() {
        let (mut c, _peer) = synced_client();
        c.pending.push((9, Pending::Devices));

        feed(
            &mut c,
            r#"{"id":9,"ok":true,"result":{"devices":[{"id":"speakers","name":"Speakers","direction":"output","state":"active"},{"id":"bt","name":"Buds","direction":"output","state":"available"}],"default_output":"bt","default_input":null}}"#,
        );

        assert_eq!(c.output_label(), "Output: Buds");
        assert_eq!(c.outputs.len(), 2);
    }

    #[test]
    fn cycling_walks_the_present_outputs_and_wraps() {
        let (mut c, mut peer) = synced_client();

        assert_eq!(c.cycle_output(), Some("Headphones".to_string()));
        let mut buf = [0u8; 256];
        let n = peer.read(&mut buf).unwrap();
        let sent = String::from_utf8_lossy(&buf[..n]).to_string();
        assert!(sent.contains(r#""command":"SetDefaultOutput""#));
        assert!(sent.contains(r#""id":"headphones""#));

        // After the switch the daemon reports it; the next press wraps.
        feed(
            &mut c,
            r#"{"event":"DefaultOutputChanged","data":{"id":"headphones"}}"#,
        );
        assert_eq!(c.cycle_output(), Some("Built-in Speakers".to_string()));
    }

    #[test]
    fn nothing_to_cycle_to_with_one_output_or_no_daemon() {
        let mut c = AudioClient::new();
        assert_eq!(c.cycle_output(), None);

        let (mut c, _peer) = connected_client();
        c.outputs.push(AudioOutput {
            id: "only".to_string(),
            name: "Only".to_string(),
        });
        assert_eq!(c.cycle_output(), None);
    }

    #[test]
    fn a_refused_action_surfaces_the_daemons_message() {
        let (mut c, _peer) = synced_client();
        c.pending.push((5, Pending::Action("Setting volume")));

        let ev = feed(
            &mut c,
            r#"{"id":5,"ok":false,"error":{"code":"invalid-volume","message":"invalid volume: 400"}}"#,
        );
        assert_eq!(
            ev,
            vec![AudioEvent::ActionFailed {
                what: "Setting volume",
                message: "invalid volume: 400".to_string()
            }]
        );
    }

    #[test]
    fn requests_use_the_daemons_wire_shape() {
        let (mut c, mut peer) = connected_client();
        assert!(c.set_volume(55));
        assert!(c.set_muted(true));
        assert!(c.set_mic_muted(true));

        let mut buf = [0u8; 512];
        let n = peer.read(&mut buf).unwrap();
        let sent = String::from_utf8_lossy(&buf[..n]).to_string();
        let lines: Vec<&str> = sent.lines().collect();
        assert_eq!(lines.len(), 3);

        let first: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["command"], "SetVolume");
        assert_eq!(first["params"]["volume"], 55);
        assert_eq!(first["id"], 1);

        let second: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["command"], "Mute");

        let third: Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(third["command"], "MuteMicrophone");
        assert_eq!(third["params"]["mute"], true);
    }

    #[test]
    fn garbage_unknown_events_and_unknown_ids_are_ignored() {
        let (mut c, _peer) = synced_client();

        assert!(feed(&mut c, "not json").is_empty());
        assert!(feed(&mut c, r#"{"event":"SomethingNew","data":{}}"#).is_empty());
        assert!(feed(&mut c, r#"{"id":77,"ok":true,"result":{}}"#).is_empty());
        assert!(feed(&mut c, r#"{"event":"StreamAdded","data":{"id":"s1","application":"a"}}"#).is_empty());
    }

    #[test]
    fn long_output_names_are_clipped_for_the_button() {
        let (mut c, _peer) = connected_client();
        c.outputs.push(AudioOutput {
            id: "x".to_string(),
            name: "A Very Long USB Audio Device Name That Won't Fit".to_string(),
        });
        c.default_output = Some("x".to_string());
        c.rebuild_label();

        assert_eq!(c.output_label().chars().count(), MAX_LABEL_CHARS);
        assert!(c.output_label().ends_with('…'));
    }

    #[test]
    fn without_a_daemon_the_label_says_so() {
        let c = AudioClient::new();
        assert_eq!(c.output_label(), "Output: n/a");
        assert!(!c.is_connected());
        assert_eq!(c.mic_muted(), None);
    }
}
