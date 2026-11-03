//! Client for mitos-bluetooth's IPC (see `src/ipc.rs` in that repo):
//! newline-delimited JSON over a Unix socket.
//!
//! What the shell gets from it: the adapter's power state (Quick Settings
//! toggle + tray icon), the *connected* devices with their battery levels
//! (Quick Settings caption, low-battery toasts), connect/disconnect toasts,
//! and pairing requests (confirm/reject prompt).
//!
//! Two things about this protocol shape the code below:
//!
//! * `subscribe` takes no filter, so the daemon sends this connection
//!   *everything*, including a `hidReport` for every key press and pointer
//!   movement of a Bluetooth keyboard or mouse. Those are dropped by a cheap
//!   substring test before any JSON is parsed (see [`is_noise`]).
//! * Event payloads name devices only by address, so events that need a
//!   human-readable name (pairing prompts) first ask `get_device`.
//!
//! Only connected devices are tracked -- discovery can surface hundreds of
//! neighbours, and nothing here ever starts a scan.

use crate::ipc_line::{log_connect_failure, Backoff, LineConn};
use serde_json::{json, Value};

/// Tried in order, unless `$MITOS_BLUETOOTH_SOCKET` names one. The daemon's
/// own default is the second; the first is where a service-managed install
/// belongs (a socket in `/tmp` can be squatted by another user -- the
/// transport refuses sockets served by anyone but root or us, but a proper
/// runtime directory avoids the question).
const DEFAULT_SOCKET_PATHS: [&str; 2] = ["/run/mitos/bluetooth.sock", "/tmp/mitos-bluetooth.sock"];

const MAX_DEVICES: usize = 32;
const MAX_PENDING: usize = 32;

/// Longest Quick Settings caption (it has to fit the panel).
const MAX_SUMMARY_CHARS: usize = 44;

/// What the user must do about a pairing request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairMethod {
    /// "Just works" / authorization: accept or reject.
    Confirm,
    /// Numeric comparison: accept if the code matches the other device.
    Compare,
    /// A code to type *on the other device*; nothing to answer here.
    ShowCode,
    /// The host must supply a PIN/passkey; this shell has no entry UI for it.
    EnterPin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BtDevice {
    pub address: String,
    pub name: String,
    pub battery: Option<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BtEvent {
    Connected,
    Disconnected,
    DeviceConnected {
        name: String,
    },
    DeviceDisconnected {
        name: String,
    },
    /// A connected device reported (or changed) its battery level.
    DeviceBattery {
        name: String,
        percent: u8,
    },
    Pairing {
        address: String,
        name: String,
        method: PairMethod,
        passkey: Option<u32>,
    },
    PairingDone {
        name: String,
        success: bool,
        error: Option<String>,
    },
    /// Something the user asked for was refused (needs privileges, ...).
    ActionFailed {
        what: &'static str,
        message: String,
    },
}

/// Why we asked `get_device` about one address.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Lookup {
    /// It connected but we hadn't seen it yet.
    Connected,
    Pairing {
        address: String,
        method: PairMethod,
        passkey: Option<u32>,
    },
    PairingDone {
        address: String,
        success: bool,
        error: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Pending {
    Adapters,
    Devices,
    Lookup(Lookup),
    /// Fire-and-forget, but a failure is worth telling the user about.
    Action(&'static str),
}

pub struct BluetoothClient {
    paths: Vec<String>,
    conn: Option<LineConn>,
    backoff: Backoff,
    next_id: u64,
    pending: Vec<(u64, Pending)>,
    line: String,
    announced_down: bool,
    lost: bool,
    /// First adapter: `(index, powered)`.
    adapter: Option<(u32, bool)>,
    /// Connected devices only.
    devices: Vec<BtDevice>,
    /// Quick Settings caption, rebuilt only when something it shows changes
    /// (it's read every frame while the panel is open).
    summary: String,
}

impl BluetoothClient {
    pub fn new() -> Self {
        let paths = match std::env::var("MITOS_BLUETOOTH_SOCKET") {
            Ok(path) if !path.is_empty() => vec![path],
            _ => DEFAULT_SOCKET_PATHS.iter().map(|p| p.to_string()).collect(),
        };

        let mut client = Self {
            paths,
            conn: None,
            backoff: Backoff::new(),
            next_id: 1,
            pending: Vec::new(),
            line: String::new(),
            announced_down: false,
            lost: false,
            adapter: None,
            devices: Vec::new(),
            summary: String::new(),
        };
        client.rebuild_summary();
        client
    }

    pub fn is_connected(&self) -> bool {
        self.conn.is_some()
    }

    /// `Some(powered)` once an adapter is known, `None` without daemon/adapter.
    pub fn adapter_powered(&self) -> Option<bool> {
        self.adapter.map(|(_, powered)| powered)
    }

    /// Quick Settings caption: connected devices (with battery) or why not.
    pub fn summary(&self) -> &str {
        &self.summary
    }

    /// Tray icon state: 0 = hidden (no adapter, or off), 1 = on, 2 = on with
    /// at least one device connected.
    pub fn tray_state(&self) -> u8 {
        match self.adapter {
            Some((_, true)) if self.conn.is_some() => {
                if self.devices.is_empty() {
                    1
                } else {
                    2
                }
            }
            _ => 0,
        }
    }

    /// Reads whatever the daemon has sent since the last call (and, while
    /// disconnected, occasionally tries to connect). Never blocks.
    pub fn poll(&mut self) -> Vec<BtEvent> {
        let mut out = Vec::new();

        if self.lost {
            self.lost = false;
            out.push(BtEvent::Disconnected);
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
            out.push(BtEvent::Disconnected);
        }

        out
    }

    // ------------------------------------------------------------------
    // Requests the GUI makes
    // ------------------------------------------------------------------

    /// Power the first adapter on or off. Both need local privileges in
    /// mitos-bluetooth (the caller must be the daemon's own user); a refusal
    /// comes back as `BtEvent::ActionFailed`. Returns false when there's no
    /// adapter to toggle.
    pub fn toggle_power(&mut self) -> bool {
        let Some((index, powered)) = self.adapter else {
            return false;
        };

        let (method, what) = if powered {
            ("power_off", "Turning Bluetooth off")
        } else {
            ("power_on", "Turning Bluetooth on")
        };

        self.request(method, json!({ "index": index }), Some(Pending::Action(what)));
        true
    }

    /// Answer a numeric-comparison / just-works pairing request.
    pub fn pairing_confirm(&mut self, address: &str, accept: bool) -> bool {
        let index = self.adapter.map(|(i, _)| i).unwrap_or(0);

        self.request(
            "pairing_confirm",
            json!({ "index": index, "address": address, "accept": accept }),
            Some(Pending::Action("Pairing")),
        )
    }

    // ------------------------------------------------------------------
    // Connection handling
    // ------------------------------------------------------------------

    fn try_connect(&mut self, out: &mut Vec<BtEvent>) {
        let mut last_err = None;
        let mut connected = None;

        for path in &self.paths {
            match LineConn::connect(path) {
                Ok(conn) => {
                    connected = Some((conn, path.clone()));
                    break;
                }
                Err(err) => last_err = Some((path.clone(), err)),
            }
        }

        let Some((conn, path)) = connected else {
            if let Some((path, err)) = last_err {
                log_connect_failure("mitos-bluetooth", &path, &err, &mut self.announced_down);
            }
            self.backoff.failed();
            return;
        };

        self.conn = Some(conn);
        self.backoff.succeeded();
        self.announced_down = false;
        tracing::info!("MITOS GUI: connected to mitos-bluetooth at {path}");

        // Subscribe first, then ask for the current state, so nothing can
        // change unseen in between.
        self.request("subscribe", json!({}), None);
        self.request("get_adapters", json!({}), Some(Pending::Adapters));

        if self.conn.is_some() {
            self.rebuild_summary();
            out.push(BtEvent::Connected);
        }
    }

    fn drop_connection(&mut self) {
        if self.conn.take().is_some() {
            tracing::warn!("MITOS GUI: lost connection to mitos-bluetooth");
        }
        self.pending.clear();
        self.adapter = None;
        self.devices.clear();
        self.backoff.lost();
        self.announced_down = false;
        self.rebuild_summary();
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

        let msg = json!({ "id": id, "method": method, "params": params });

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

    fn lookup(&mut self, address: &str, why: Lookup) {
        let index = self.adapter.map(|(i, _)| i).unwrap_or(0);

        self.request(
            "get_device",
            json!({ "index": index, "address": address }),
            Some(Pending::Lookup(why)),
        );
    }

    // ------------------------------------------------------------------
    // Incoming lines
    // ------------------------------------------------------------------

    fn handle_line(&mut self, line: &str, out: &mut Vec<BtEvent>) {
        // Events are `{"event":{...}}`; replies carry an `id` and `ok`.
        if line.starts_with(r#"{"event":"#) && is_noise(line) {
            return;
        }

        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
        };

        if let Some(ev) = v.get("event") {
            self.handle_event(ev, out);
        } else if v.get("ok").is_some() {
            self.handle_response(&v, out);
        }
    }

    fn handle_event(&mut self, ev: &Value, out: &mut Vec<BtEvent>) {
        let Some(kind) = ev.get("type").and_then(Value::as_str) else {
            return;
        };
        let data = ev.get("data").unwrap_or(&Value::Null);

        match kind {
            "deviceUpdated" => {
                if let Some(dev) = data.get("device").and_then(parse_device) {
                    self.apply_device(dev, out);
                }
            }
            "deviceConnected" => {
                if let Some(address) = data.get("id").and_then(address_of) {
                    if !self.devices.iter().any(|d| d.address == address) {
                        self.lookup(&address, Lookup::Connected);
                    }
                }
            }
            "deviceDisconnected" => {
                if let Some(address) = data.get("id").and_then(address_of) {
                    self.remove_device(&address, out);
                }
            }
            "batteryChanged" => {
                let address = data.get("id").and_then(address_of);
                let percent = data
                    .get("percent")
                    .and_then(Value::as_u64)
                    .map(|p| p.min(100) as u8);

                if let (Some(address), Some(percent)) = (address, percent) {
                    self.set_battery(&address, percent, out);
                }
            }
            "adapterStateChanged" => {
                let index = data.get("index").and_then(Value::as_u64).map(|i| i as u32);
                let powered = data.get("powered").and_then(Value::as_bool);

                if let (Some(index), Some(powered)) = (index, powered) {
                    let ours = match self.adapter {
                        Some((i, _)) => i == index,
                        None => true,
                    };
                    if ours {
                        self.adapter = Some((index, powered));
                        if !powered {
                            // Everything drops with the adapter.
                            self.devices.clear();
                        }
                        self.rebuild_summary();
                    }
                }
            }
            "adapterAdded" => {
                self.request("get_adapters", json!({}), Some(Pending::Adapters));
            }
            "adapterRemoved" => {
                let index = data.get("index").and_then(Value::as_u64).map(|i| i as u32);
                if index.is_some() && self.adapter.map(|(i, _)| i) == index {
                    self.adapter = None;
                    self.devices.clear();
                    self.rebuild_summary();
                }
            }
            "pairingRequested" => {
                let request = data.get("request").unwrap_or(&Value::Null);

                if let Some(address) = request.get("device").and_then(address_of) {
                    let passkey = request
                        .get("passkey")
                        .and_then(Value::as_u64)
                        .map(|k| k as u32);
                    let method = pair_method(
                        request.get("method").and_then(Value::as_str).unwrap_or(""),
                        passkey,
                    );

                    self.lookup(
                        &address.clone(),
                        Lookup::Pairing {
                            address,
                            method,
                            passkey,
                        },
                    );
                }
            }
            "pairingComplete" => {
                if let Some(address) = data.get("id").and_then(address_of) {
                    let success = data.get("success").and_then(Value::as_bool).unwrap_or(false);
                    let error = data
                        .get("error")
                        .and_then(Value::as_str)
                        .map(str::to_string);

                    self.lookup(
                        &address.clone(),
                        Lookup::PairingDone {
                            address,
                            success,
                            error,
                        },
                    );
                }
            }
            _ => {}
        }
    }

    fn handle_response(&mut self, v: &Value, out: &mut Vec<BtEvent>) {
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
                .and_then(Value::as_str)
                .unwrap_or("unknown error")
                .to_string();

            match kind {
                Pending::Action(what) => out.push(BtEvent::ActionFailed { what, message }),
                // The lookup only wanted a name: fall back to the address
                // rather than losing the prompt.
                Pending::Lookup(why) => self.finish_lookup(why, None, out),
                _ => {}
            }
            return;
        }

        let result = v.get("result").unwrap_or(&Value::Null);

        match kind {
            Pending::Adapters => {
                self.adapter = result
                    .as_array()
                    .and_then(|list| list.first())
                    .and_then(parse_adapter);

                if let Some((index, _)) = self.adapter {
                    self.request("get_devices", json!({ "index": index }), Some(Pending::Devices));
                } else {
                    self.devices.clear();
                }
                self.rebuild_summary();
            }
            Pending::Devices => {
                self.devices.clear();

                if let Some(list) = result.as_array() {
                    for item in list {
                        if let Some(dev) = parse_device(item) {
                            if dev.connected && self.devices.len() < MAX_DEVICES {
                                self.devices.push(BtDevice {
                                    address: dev.address,
                                    name: dev.name,
                                    battery: dev.battery,
                                });
                            }
                        }
                    }
                }
                self.rebuild_summary();
            }
            Pending::Lookup(why) => {
                let dev = parse_device(result);
                self.finish_lookup(why, dev, out);
            }
            Pending::Action(_) => {}
        }
    }

    fn finish_lookup(&mut self, why: Lookup, dev: Option<ParsedDevice>, out: &mut Vec<BtEvent>) {
        match why {
            Lookup::Connected => {
                if let Some(dev) = dev {
                    self.apply_device(dev, out);
                }
            }
            Lookup::Pairing {
                address,
                method,
                passkey,
            } => {
                let name = dev.map(|d| d.name).unwrap_or_else(|| address.clone());
                out.push(BtEvent::Pairing {
                    address,
                    name,
                    method,
                    passkey,
                });
            }
            Lookup::PairingDone {
                address,
                success,
                error,
            } => {
                let name = dev.map(|d| d.name).unwrap_or(address);
                out.push(BtEvent::PairingDone {
                    name,
                    success,
                    error,
                });
            }
        }
    }

    // ------------------------------------------------------------------
    // Device table
    // ------------------------------------------------------------------

    /// Folds a full device snapshot into the table and reports what changed.
    fn apply_device(&mut self, dev: ParsedDevice, out: &mut Vec<BtEvent>) {
        let existing = self.devices.iter().position(|d| d.address == dev.address);

        match (dev.connected, existing) {
            (true, None) => {
                if self.devices.len() < MAX_DEVICES {
                    self.devices.push(BtDevice {
                        address: dev.address.clone(),
                        name: dev.name.clone(),
                        battery: dev.battery,
                    });
                }
                out.push(BtEvent::DeviceConnected {
                    name: dev.name.clone(),
                });
                if let Some(percent) = dev.battery {
                    out.push(BtEvent::DeviceBattery {
                        name: dev.name,
                        percent,
                    });
                }
            }
            (true, Some(i)) => {
                let known = &mut self.devices[i];
                known.name = dev.name.clone();

                if known.battery != dev.battery {
                    known.battery = dev.battery;
                    if let Some(percent) = dev.battery {
                        out.push(BtEvent::DeviceBattery {
                            name: dev.name,
                            percent,
                        });
                    }
                }
            }
            (false, Some(_)) => {
                self.remove_device(&dev.address, out);
                return;
            }
            (false, None) => return,
        }

        self.rebuild_summary();
    }

    fn remove_device(&mut self, address: &str, out: &mut Vec<BtEvent>) {
        if let Some(i) = self.devices.iter().position(|d| d.address == address) {
            let gone = self.devices.remove(i);
            out.push(BtEvent::DeviceDisconnected { name: gone.name });
            self.rebuild_summary();
        }
    }

    fn set_battery(&mut self, address: &str, percent: u8, out: &mut Vec<BtEvent>) {
        if let Some(i) = self.devices.iter().position(|d| d.address == address) {
            if self.devices[i].battery != Some(percent) {
                self.devices[i].battery = Some(percent);
                out.push(BtEvent::DeviceBattery {
                    name: self.devices[i].name.clone(),
                    percent,
                });
                self.rebuild_summary();
            }
        }
    }

    fn rebuild_summary(&mut self) {
        let text = if self.conn.is_none() {
            "Bluetooth service not running".to_string()
        } else {
            match self.adapter {
                None => "No Bluetooth adapter".to_string(),
                Some((_, false)) => "Off".to_string(),
                Some((_, true)) if self.devices.is_empty() => "On - no devices connected".to_string(),
                Some((_, true)) => self
                    .devices
                    .iter()
                    .map(|d| match d.battery {
                        Some(p) => format!("{} {}%", d.name, p),
                        None => d.name.clone(),
                    })
                    .collect::<Vec<_>>()
                    .join(" · "),
            }
        };

        self.summary = clip(&text, MAX_SUMMARY_CHARS);
    }
}

// ----------------------------------------------------------------------
// Parsing
// ----------------------------------------------------------------------

/// Event types that are never useful here and arrive constantly: HID reports
/// (one per key press / pointer movement of a Bluetooth input device), GATT
/// notifications, discovery chatter and audio plumbing. Matched as
/// substrings of the raw line so none of them is ever parsed.
const NOISE: [&str; 11] = [
    r#""type":"hidReport""#,
    r#""type":"gattNotification""#,
    r#""type":"deviceFound""#,
    r#""type":"deviceLost""#,
    r#""type":"discoveryStarted""#,
    r#""type":"discoveryStopped""#,
    r#""type":"a2dpStateChanged""#,
    r#""type":"a2dpStreamReady""#,
    r#""type":"scoStateChanged""#,
    r#""type":"audioVolumeChanged""#,
    r#""type":"mediaCommand""#,
];

/// Whether an *event* line is one of the types in [`NOISE`]. Only ever call
/// this on event lines: replies legitimately contain words like `a2dp`.
fn is_noise(event_line: &str) -> bool {
    NOISE.iter().any(|pattern| event_line.contains(*pattern))
}

struct ParsedDevice {
    address: String,
    name: String,
    connected: bool,
    battery: Option<u8>,
}

/// `{"address": "AA:BB:...", "type": "bredr"}` -> the address.
fn address_of(id: &Value) -> Option<String> {
    id.get("address")?.as_str().map(str::to_string)
}

fn prop_str<'a>(device: &'a Value, key: &str) -> Option<&'a str> {
    device
        .get("properties")?
        .get(key)?
        .as_str()
        .filter(|s| !s.is_empty())
}

fn parse_device(d: &Value) -> Option<ParsedDevice> {
    let address = address_of(d.get("id")?)?;

    let name = prop_str(d, "alias")
        .or_else(|| prop_str(d, "name"))
        .map(clean_name)
        .unwrap_or_else(|| address.clone());

    let connected = d.get("connected").and_then(Value::as_bool).unwrap_or(false);
    let battery = d
        .get("battery")
        .and_then(Value::as_u64)
        .map(|p| p.min(100) as u8);

    Some(ParsedDevice {
        address,
        name,
        connected,
        battery,
    })
}

fn parse_adapter(a: &Value) -> Option<(u32, bool)> {
    let index = a.get("index").and_then(Value::as_u64)? as u32;
    let powered = a.get("powered").and_then(Value::as_bool).unwrap_or(false);
    Some((index, powered))
}

fn pair_method(method: &str, passkey: Option<u32>) -> PairMethod {
    match method {
        "numericComparison" => PairMethod::Compare,
        "passkeyEntry" => {
            if passkey.is_some() {
                PairMethod::ShowCode
            } else {
                PairMethod::EnterPin
            }
        }
        "pinCode" => PairMethod::EnterPin,
        // justWorks, authorization
        _ => PairMethod::Confirm,
    }
}

/// Device names come from the device itself -- strip anything that isn't
/// plain printable text before it reaches a toast.
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

    fn feed(c: &mut BluetoothClient, text: &str) -> Vec<BtEvent> {
        let mut out = Vec::new();
        c.handle_line(text, &mut out);
        out
    }

    /// A client wired to one end of a socketpair, with a powered adapter.
    fn connected_client() -> (BluetoothClient, UnixStream) {
        let (a, b) = UnixStream::pair().unwrap();
        let mut c = BluetoothClient::new();
        c.conn = Some(LineConn::from_stream(a).unwrap());
        c.adapter = Some((0, true));
        c.rebuild_summary();
        (c, b)
    }

    const BUDS_CONNECTED: &str = r#"{"event":{"data":{"device":{"id":{"address":"11:22:33:44:55:66","type":"bredr"},"properties":{"alias":"Pixel Buds","name":"Buds","class":0,"uuids":[]},"state":"connected","paired":true,"trusted":true,"connected":true,"battery":80,"profiles":["a2dp"],"lastSeen":1,"lastConnected":1}},"type":"deviceUpdated"}}"#;

    #[test]
    fn noise_filter_drops_hid_reports_but_not_replies() {
        let hid = r#"{"event":{"data":{"id":{"address":"AA:BB:CC:DD:EE:FF","type":"bredr"},"input":{"mouse":{"buttons":0,"dx":3,"dy":-1,"wheel":0}}},"type":"hidReport"}}"#;
        assert!(is_noise(hid));
        assert!(!is_noise(BUDS_CONNECTED));

        // A reply that merely mentions a2dp must not be mistaken for noise
        // (handle_line only applies the filter to event lines).
        let (mut c, _peer) = connected_client();
        c.pending.push((5, Pending::Devices));
        let reply = r#"{"id":5,"ok":true,"result":[{"id":{"address":"11:22:33:44:55:66","type":"bredr"},"properties":{"alias":"Buds"},"connected":true,"battery":55,"profiles":["a2dp"]}]}"#;
        feed(&mut c, reply);
        assert_eq!(c.devices.len(), 1);
        assert_eq!(c.devices[0].battery, Some(55));
    }

    #[test]
    fn connecting_a_device_reports_it_once_with_its_battery() {
        let (mut c, _peer) = connected_client();

        let ev = feed(&mut c, BUDS_CONNECTED);
        assert_eq!(
            ev,
            vec![
                BtEvent::DeviceConnected {
                    name: "Pixel Buds".to_string(),
                },
                BtEvent::DeviceBattery {
                    name: "Pixel Buds".to_string(),
                    percent: 80,
                },
            ]
        );
        assert_eq!(c.summary(), "Pixel Buds 80%");
        assert_eq!(c.tray_state(), 2);

        // The same update again changes nothing.
        assert!(feed(&mut c, BUDS_CONNECTED).is_empty());
    }

    #[test]
    fn battery_events_update_the_table_and_report_changes_only() {
        let (mut c, _peer) = connected_client();
        feed(&mut c, BUDS_CONNECTED);

        let ev = feed(
            &mut c,
            r#"{"event":{"data":{"id":{"address":"11:22:33:44:55:66","type":"bredr"},"percent":12},"type":"batteryChanged"}}"#,
        );
        assert_eq!(
            ev,
            vec![BtEvent::DeviceBattery {
                name: "Pixel Buds".to_string(),
                percent: 12,
            }]
        );
        assert_eq!(c.summary(), "Pixel Buds 12%");

        // Same level again, and a device we never saw: silence.
        assert!(feed(
            &mut c,
            r#"{"event":{"data":{"id":{"address":"11:22:33:44:55:66","type":"bredr"},"percent":12},"type":"batteryChanged"}}"#
        )
        .is_empty());
        assert!(feed(
            &mut c,
            r#"{"event":{"data":{"id":{"address":"99:99:99:99:99:99","type":"bredr"},"percent":5},"type":"batteryChanged"}}"#
        )
        .is_empty());
    }

    #[test]
    fn disconnect_removes_the_device_and_reports_its_name() {
        let (mut c, _peer) = connected_client();
        feed(&mut c, BUDS_CONNECTED);

        let ev = feed(
            &mut c,
            r#"{"event":{"data":{"id":{"address":"11:22:33:44:55:66","type":"bredr"},"reason":"remote"},"type":"deviceDisconnected"}}"#,
        );
        assert_eq!(
            ev,
            vec![BtEvent::DeviceDisconnected {
                name: "Pixel Buds".to_string(),
            }]
        );
        assert_eq!(c.summary(), "On - no devices connected");
        assert_eq!(c.tray_state(), 1);
    }

    #[test]
    fn powering_the_adapter_off_clears_devices_and_summary() {
        let (mut c, _peer) = connected_client();
        feed(&mut c, BUDS_CONNECTED);

        feed(
            &mut c,
            r#"{"event":{"data":{"index":0,"powered":false,"discoverable":false,"connectable":false},"type":"adapterStateChanged"}}"#,
        );
        assert_eq!(c.adapter_powered(), Some(false));
        assert_eq!(c.summary(), "Off");
        assert_eq!(c.tray_state(), 0);
    }

    #[test]
    fn pairing_request_looks_up_the_name_before_prompting() {
        let (mut c, mut peer) = connected_client();

        let ev = feed(
            &mut c,
            r#"{"event":{"data":{"request":{"device":{"address":"AA:BB:CC:DD:EE:FF","type":"bredr"},"method":"numericComparison","passkey":123456}},"type":"pairingRequested"}}"#,
        );
        assert!(ev.is_empty()); // waiting for the device's name

        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut buf = [0u8; 512];
        let n = peer.read(&mut buf).unwrap();
        let sent = String::from_utf8_lossy(&buf[..n]).to_string();
        assert!(sent.contains(r#""method":"get_device""#));
        assert!(sent.contains("AA:BB:CC:DD:EE:FF"));

        let id = c.pending[0].0;
        let reply = r#"{"id":@ID@,"ok":true,"result":{"id":{"address":"AA:BB:CC:DD:EE:FF","type":"bredr"},"properties":{"alias":"Speaker\u0007","name":"Spk"},"connected":false,"battery":null}}"#
            .replace("@ID@", &id.to_string());

        let ev = feed(&mut c, &reply);
        assert_eq!(
            ev,
            vec![BtEvent::Pairing {
                address: "AA:BB:CC:DD:EE:FF".to_string(),
                name: "Speaker".to_string(), // control character stripped
                method: PairMethod::Compare,
                passkey: Some(123456),
            }]
        );
    }

    #[test]
    fn pairing_falls_back_to_the_address_when_the_lookup_fails() {
        let (mut c, _peer) = connected_client();

        feed(
            &mut c,
            r#"{"event":{"data":{"request":{"device":{"address":"AA:BB:CC:DD:EE:FF","type":"bredr"},"method":"justWorks","passkey":null}},"type":"pairingRequested"}}"#,
        );

        let id = c.pending[0].0;
        let reply = r#"{"id":@ID@,"ok":false,"error":"not found"}"#.replace("@ID@", &id.to_string());

        assert_eq!(
            feed(&mut c, &reply),
            vec![BtEvent::Pairing {
                address: "AA:BB:CC:DD:EE:FF".to_string(),
                name: "AA:BB:CC:DD:EE:FF".to_string(),
                method: PairMethod::Confirm,
                passkey: None,
            }]
        );
    }

    #[test]
    fn pairing_methods_map_to_what_the_user_must_do() {
        assert_eq!(pair_method("justWorks", None), PairMethod::Confirm);
        assert_eq!(pair_method("authorization", None), PairMethod::Confirm);
        assert_eq!(pair_method("numericComparison", Some(1)), PairMethod::Compare);
        assert_eq!(pair_method("passkeyEntry", Some(1)), PairMethod::ShowCode);
        assert_eq!(pair_method("passkeyEntry", None), PairMethod::EnterPin);
        assert_eq!(pair_method("pinCode", None), PairMethod::EnterPin);
    }

    #[test]
    fn adapter_reply_starts_the_device_listing() {
        let (mut c, mut peer) = connected_client();
        c.adapter = None;
        c.pending.push((1, Pending::Adapters));

        feed(
            &mut c,
            r#"{"id":1,"ok":true,"result":[{"index":0,"address":"00:11:22:33:44:55","name":"hci0","powered":true,"leSupported":true}]}"#,
        );
        assert_eq!(c.adapter_powered(), Some(true));

        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut buf = [0u8; 256];
        let n = peer.read(&mut buf).unwrap();
        assert!(String::from_utf8_lossy(&buf[..n]).contains(r#""method":"get_devices""#));
    }

    #[test]
    fn long_summaries_are_clipped_to_fit_the_panel() {
        let (mut c, _peer) = connected_client();
        c.devices.push(BtDevice {
            address: "1".to_string(),
            name: "A Rather Long Headphone Name Indeed".to_string(),
            battery: Some(100),
        });
        c.devices.push(BtDevice {
            address: "2".to_string(),
            name: "Another Device".to_string(),
            battery: Some(50),
        });
        c.rebuild_summary();

        assert_eq!(c.summary().chars().count(), MAX_SUMMARY_CHARS);
        assert!(c.summary().ends_with('…'));
    }

    #[test]
    fn without_a_daemon_the_caption_says_so() {
        let c = BluetoothClient::new();
        assert_eq!(c.summary(), "Bluetooth service not running");
        assert_eq!(c.tray_state(), 0);
        assert_eq!(c.adapter_powered(), None);
    }
}
