//! Client for mitos-input's IPC (see its README, "IPC protocol"):
//! newline-delimited *text* over `/run/mitos/input.sock`.
//!
//! What mitos-gui takes from mitos-input, and what it deliberately doesn't:
//!
//! * **Device inventory + hot-plug** (`LIST-DEVICES`, `DEVICE-ADDED`,
//!   `DEVICE-REMOVED`) -- so the shell knows whether a keyboard or a
//!   touchscreen is attached, and can say so when one appears.
//! * **Touchscreen gestures** (`GESTURE-SWIPE`) -- libinput delivers
//!   multi-finger gestures for touchpads only, and `input.rs` ignores touch
//!   events, so mitos-input's recognizer is the one source of touchscreen
//!   gestures.
//! * **Not** the raw `KEY`/`POINTER` stream. libinput (through libseat)
//!   already feeds the compositor those directly; relaying every keystroke
//!   through a socket would add a hop and latency, and make typing depend on
//!   another daemon being alive.
//! * **Not** `HOTKEY`. `keyboard.rs` already matches the same shortcuts, so
//!   acting on both would fire every shortcut twice.
//!
//! The socket is `0600` by default (anyone who can read it sees every
//! keystroke), so mitos-gui must run as the same user as mitos-input or be
//! let in with a deliberately widened socket mode.

use crate::ipc_line::{log_connect_failure, Backoff, LineConn};

const DEFAULT_SOCKET_PATH: &str = "/run/mitos/input.sock";

/// Devices tracked. More than this is a misbehaving daemon, not a machine.
const MAX_DEVICES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Keyboard,
    Mouse,
    Touchpad,
    Touchscreen,
    Tablet,
    Gamepad,
    Switch,
    Unknown,
}

impl DeviceKind {
    fn parse(s: &str) -> Self {
        match s {
            "keyboard" => Self::Keyboard,
            "mouse" => Self::Mouse,
            "touchpad" => Self::Touchpad,
            "touchscreen" => Self::Touchscreen,
            "tablet" => Self::Tablet,
            "gamepad" => Self::Gamepad,
            "switch" => Self::Switch,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Keyboard => "Keyboard",
            Self::Mouse => "Mouse",
            Self::Touchpad => "Touchpad",
            Self::Touchscreen => "Touchscreen",
            Self::Tablet => "Drawing tablet",
            Self::Gamepad => "Game controller",
            Self::Switch => "Input switch",
            Self::Unknown => "Input device",
        }
    }

    /// Worth a toast when it appears/disappears (lid switches and
    /// unclassified nodes aren't).
    pub fn is_notable(self) -> bool {
        !matches!(self, Self::Switch | Self::Unknown)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwipeDir {
    Up,
    Down,
    Left,
    Right,
}

impl SwipeDir {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "up" => Some(Self::Up),
            "down" => Some(Self::Down),
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    Connected,
    Disconnected,
    /// `initial` is true for devices from the startup `LIST-DEVICES` (nothing
    /// was *plugged in*), false for real hot-plug.
    DeviceAdded {
        kind: DeviceKind,
        name: String,
        initial: bool,
    },
    DeviceRemoved {
        kind: DeviceKind,
        name: String,
    },
    /// The startup device list is complete.
    Listed,
    Swipe {
        from: DeviceKind,
        fingers: u32,
        dir: SwipeDir,
    },
}

struct Device {
    id: u32,
    kind: DeviceKind,
    name: String,
}

pub struct InputClient {
    path: String,
    conn: Option<LineConn>,
    backoff: Backoff,
    line: String,
    devices: Vec<Device>,
    announced_down: bool,
}

impl InputClient {
    pub fn new() -> Self {
        let path = std::env::var("MITOS_INPUT_SOCKET")
            .unwrap_or_else(|_| DEFAULT_SOCKET_PATH.to_string());

        Self {
            path,
            conn: None,
            backoff: Backoff::new(),
            line: String::new(),
            devices: Vec::new(),
            announced_down: false,
        }
    }

    /// Whether a device of this kind is currently attached.
    pub fn has(&self, kind: DeviceKind) -> bool {
        self.devices.iter().any(|d| d.kind == kind)
    }

    /// Reads whatever mitos-input has sent since the last call (and, while
    /// disconnected, occasionally tries to connect). Never blocks.
    pub fn poll(&mut self) -> Vec<InputEvent> {
        let mut out = Vec::new();

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
            if self.conn.take().is_some() {
                tracing::warn!("MITOS GUI: lost connection to mitos-input");
            }
            self.devices.clear();
            self.backoff.lost();
            self.announced_down = false;
            out.push(InputEvent::Disconnected);
        }

        out
    }

    fn try_connect(&mut self, out: &mut Vec<InputEvent>) {
        match LineConn::connect(&self.path) {
            Ok(mut conn) => {
                // Subscribe first: once `OK` is back no later hot-plug can be
                // missed, and a device that shows up between the two commands
                // is simply seen twice (the second sighting is ignored).
                let sent = conn.send(b"SUBSCRIBE device,gesture\nLIST-DEVICES\n");
                if !sent {
                    self.backoff.failed();
                    return;
                }

                self.conn = Some(conn);
                self.devices.clear();
                self.backoff.succeeded();
                self.announced_down = false;
                tracing::info!("MITOS GUI: connected to mitos-input at {}", self.path);
                out.push(InputEvent::Connected);
            }
            Err(err) => {
                log_connect_failure("mitos-input", &self.path, &err, &mut self.announced_down);
                self.backoff.failed();
            }
        }
    }

    fn handle_line(&mut self, line: &str, out: &mut Vec<InputEvent>) {
        let toks = tokenize(line);
        let Some(head) = toks.first().copied() else {
            return;
        };
        let args = &toks[1..];

        match head {
            "DEVICE" | "DEVICE-ADDED" => {
                let initial = head == "DEVICE";

                if let Some((id, kind, name)) = parse_device(args) {
                    if self.devices.iter().any(|d| d.id == id) {
                        return;
                    }
                    if self.devices.len() < MAX_DEVICES {
                        self.devices.push(Device {
                            id,
                            kind,
                            name: name.clone(),
                        });
                    }
                    out.push(InputEvent::DeviceAdded { kind, name, initial });
                }
            }
            "DEVICE-REMOVED" => {
                let id: Option<u32> = field(args, "id").and_then(|s| s.parse().ok());

                if let Some(id) = id {
                    if let Some(pos) = self.devices.iter().position(|d| d.id == id) {
                        let gone = self.devices.remove(pos);
                        out.push(InputEvent::DeviceRemoved {
                            kind: gone.kind,
                            name: gone.name,
                        });
                    }
                }
            }
            "END" => out.push(InputEvent::Listed),
            "GESTURE-SWIPE" => {
                let dev: Option<u32> = field(args, "dev").and_then(|s| s.parse().ok());
                let fingers: u32 = field(args, "fingers")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let dir = field(args, "dir").and_then(SwipeDir::parse);

                if let (Some(dev), Some(dir)) = (dev, dir) {
                    let from = self
                        .devices
                        .iter()
                        .find(|d| d.id == dev)
                        .map(|d| d.kind)
                        .unwrap_or(DeviceKind::Unknown);

                    out.push(InputEvent::Swipe { from, fingers, dir });
                }
            }
            "ERR" => {
                tracing::debug!("MITOS GUI: mitos-input refused a command: {line}");
            }
            // OK / PONG, and every kind we didn't subscribe to.
            _ => {}
        }
    }
}

// ----------------------------------------------------------------------
// Parsing
// ----------------------------------------------------------------------

/// Splits a protocol line on spaces, keeping `"quoted values"` (device names
/// contain spaces) in one piece, escapes included.
fn tokenize(line: &str) -> Vec<&str> {
    let mut toks = Vec::new();
    let mut start: Option<usize> = None;
    let mut in_quote = false;
    let mut escaped = false;

    for (i, &b) in line.as_bytes().iter().enumerate() {
        if in_quote {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_quote = false;
            }
            continue;
        }

        if b == b' ' || b == b'\t' {
            if let Some(s) = start.take() {
                toks.push(&line[s..i]);
            }
        } else {
            if start.is_none() {
                start = Some(i);
            }
            if b == b'"' {
                in_quote = true;
            }
        }
    }

    if let Some(s) = start {
        toks.push(&line[s..]);
    }

    toks
}

/// The value of `key=value` among `toks`.
fn field<'a>(toks: &[&'a str], key: &str) -> Option<&'a str> {
    toks.iter().copied().find_map(|t| {
        let rest = t.strip_prefix(key)?;
        rest.strip_prefix('=')
    })
}

/// Undoes mitos-input's `quote()`: `\"`, `\\`, `\xNN` and `\u{NNNN}`.
fn unquote(raw: &str) -> String {
    let inner = match raw.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        Some(inner) => inner,
        None => return raw.to_string(),
    };

    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();

    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }

        match chars.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('x') => {
                let hex: String = chars.by_ref().take(2).collect();
                match u8::from_str_radix(&hex, 16) {
                    Ok(byte) => out.push(byte as char),
                    Err(_) => out.push('?'),
                }
            }
            Some('u') => {
                if chars.next() == Some('{') {
                    let mut hex = String::new();
                    for h in chars.by_ref() {
                        if h == '}' {
                            break;
                        }
                        hex.push(h);
                    }
                    match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        Some(ch) => out.push(ch),
                        None => out.push('?'),
                    }
                } else {
                    out.push('?');
                }
            }
            Some(other) => out.push(other),
            None => {}
        }
    }

    out
}

/// A device name safe to put in a toast: no control characters, bounded.
fn display_name(raw: &str) -> String {
    unquote(raw)
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect()
}

fn parse_device(args: &[&str]) -> Option<(u32, DeviceKind, String)> {
    let id: u32 = field(args, "id")?.parse().ok()?;
    let kind = DeviceKind::parse(field(args, "type").unwrap_or("unknown"));
    let name = field(args, "name").map(display_name).unwrap_or_default();

    Some((id, kind, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> InputClient {
        InputClient::new()
    }

    fn feed(c: &mut InputClient, text: &str) -> Vec<InputEvent> {
        let mut out = Vec::new();
        c.handle_line(text, &mut out);
        out
    }

    #[test]
    fn tokenizer_keeps_quoted_values_together() {
        let toks = tokenize(
            r#"DEVICE-ADDED id=4 type=keyboard name="USB \"Pro\" Keyboard" vendor=0x046d node="/dev/input/event4""#,
        );
        assert_eq!(toks[0], "DEVICE-ADDED");
        assert_eq!(field(&toks[1..], "id"), Some("4"));
        assert_eq!(field(&toks[1..], "vendor"), Some("0x046d"));
        assert_eq!(
            unquote(field(&toks[1..], "name").unwrap()),
            r#"USB "Pro" Keyboard"#
        );
        assert_eq!(
            unquote(field(&toks[1..], "node").unwrap()),
            "/dev/input/event4"
        );
    }

    #[test]
    fn unquote_handles_hex_and_unicode_escapes() {
        assert_eq!(unquote(r#""a\x0ab""#), "a\nb");
        assert_eq!(unquote(r#""caf\u{00e9}""#), "caf\u{e9}");
        assert_eq!(unquote("plain"), "plain");
    }

    #[test]
    fn names_lose_control_characters_and_are_bounded() {
        assert_eq!(display_name(r#""evil\x0aname""#), "evilname");
        let long = format!("\"{}\"", "x".repeat(200));
        assert_eq!(display_name(&long).len(), 64);
    }

    #[test]
    fn startup_list_is_initial_and_hotplug_is_not() {
        let mut c = client();

        let ev = feed(
            &mut c,
            r#"DEVICE id=3 type=keyboard name="AT Keyboard" vendor=0x0001 product=0x0001 bus=0x0011 seat="seat0" caps=0x1 node="/dev/input/event3""#,
        );
        assert_eq!(
            ev,
            vec![InputEvent::DeviceAdded {
                kind: DeviceKind::Keyboard,
                name: "AT Keyboard".to_string(),
                initial: true,
            }]
        );
        assert_eq!(feed(&mut c, "END"), vec![InputEvent::Listed]);
        assert!(c.has(DeviceKind::Keyboard));

        let ev = feed(
            &mut c,
            r#"DEVICE-ADDED id=9 type=touchscreen name="Goodix" vendor=0x0000 product=0x0000 bus=0x0018 seat="seat0" caps=0x4 node="-""#,
        );
        assert_eq!(
            ev,
            vec![InputEvent::DeviceAdded {
                kind: DeviceKind::Touchscreen,
                name: "Goodix".to_string(),
                initial: false,
            }]
        );
        assert!(c.has(DeviceKind::Touchscreen));

        let ev = feed(&mut c, "DEVICE-REMOVED id=9");
        assert_eq!(
            ev,
            vec![InputEvent::DeviceRemoved {
                kind: DeviceKind::Touchscreen,
                name: "Goodix".to_string(),
            }]
        );
        assert!(!c.has(DeviceKind::Touchscreen));
    }

    #[test]
    fn a_device_seen_twice_is_reported_once() {
        let mut c = client();
        let line = r#"DEVICE id=3 type=mouse name="M" vendor=0x1 product=0x1 bus=0x3 seat="seat0" caps=0x1 node="-""#;
        assert_eq!(feed(&mut c, line).len(), 1);
        assert!(feed(&mut c, &line.replacen("DEVICE ", "DEVICE-ADDED ", 1)).is_empty());
    }

    #[test]
    fn swipes_carry_the_source_device_kind() {
        let mut c = client();
        feed(
            &mut c,
            r#"DEVICE id=7 type=touchscreen name="TS" vendor=0x1 product=0x1 bus=0x18 seat="seat0" caps=0x4 node="-""#,
        );

        let ev = feed(
            &mut c,
            "GESTURE-SWIPE dev=7 t=12.900000 fingers=3 dir=up dx=0.01000 dy=-0.08000",
        );
        assert_eq!(
            ev,
            vec![InputEvent::Swipe {
                from: DeviceKind::Touchscreen,
                fingers: 3,
                dir: SwipeDir::Up,
            }]
        );

        // A device we never heard of is Unknown, not a crash.
        let ev = feed(&mut c, "GESTURE-SWIPE dev=99 t=1.0 fingers=4 dir=left dx=-0.1 dy=0.0");
        assert_eq!(
            ev,
            vec![InputEvent::Swipe {
                from: DeviceKind::Unknown,
                fingers: 4,
                dir: SwipeDir::Left,
            }]
        );
    }

    #[test]
    fn acks_errors_and_garbage_are_ignored() {
        let mut c = client();
        assert!(feed(&mut c, "OK").is_empty());
        assert!(feed(&mut c, "PONG").is_empty());
        assert!(feed(&mut c, "ERR unknown event kind").is_empty());
        assert!(feed(&mut c, "").is_empty());
        assert!(feed(&mut c, "GESTURE-SWIPE dev=x dir=sideways").is_empty());
    }
}
