//! Client connection to mitos-session's IPC socket.
//!
//! mitos-session owns every lock/idle/auth *decision*; mitos-gui only
//! draws what it's told to. This module's whole job is ferrying
//! `mitos_session::ipc` messages between that daemon and the render
//! loop -- see `docs/architecture.md` and `docs/security.md` in
//! mitos-session for the design this implements.
//!
//! Mirrors `dbus.rs`'s shape: a background thread blocks on socket
//! reads and forwards decoded messages over an `mpsc` channel, which
//! the main loop drains once per frame via `MitosGuiState::poll_session_ipc`.
//! Outgoing requests are written directly from the main thread on a
//! cloned handle to the same socket.

use mitos_session::ipc::{read_message, write_message, Message, Request, Response};
use std::os::unix::net::UnixStream;
use std::sync::mpsc;

/// Default listen path from mitos-session's own `config/defaults.rs`.
/// Overridable via `$MITOS_SESSION_SOCKET` in case a deployment moves
/// it (matches mitos-session's own `MITOS_SESSION_CONFIG` convention
/// of "env var overrides the built-in default").
const DEFAULT_SOCKET_PATH: &str = "/run/mitos-session/session.sock";

pub struct SessionIpc {
    write_half: UnixStream,
    pub rx: mpsc::Receiver<Message>,
    pub session_id: u32,
}

impl SessionIpc {
    /// Connect to mitos-session and register this process as the
    /// compositor for the current session.
    ///
    /// Returns `None` (never an error the caller has to handle) when
    /// there's nothing to connect to -- e.g. `mitos-gui` launched by
    /// hand for development, outside a real mitos-session-managed
    /// session, where `$XDG_SESSION_ID` won't be set. Lock-screen
    /// support is simply unavailable in that case; everything else
    /// about the compositor still works.
    pub fn connect() -> Option<Self> {
        let session_id: u32 = std::env::var("XDG_SESSION_ID").ok()?.parse().ok()?;

        let socket_path = std::env::var("MITOS_SESSION_SOCKET")
            .unwrap_or_else(|_| DEFAULT_SOCKET_PATH.to_string());

        let mut stream = match UnixStream::connect(&socket_path) {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!("MITOS GUI: could not connect to mitos-session at {socket_path}: {err} -- lock screen support disabled");
                return None;
            }
        };

        if let Err(err) = write_message(&mut stream, &Request::RegisterCompositor { session_id }) {
            tracing::warn!("MITOS GUI: could not register with mitos-session: {err}");
            return None;
        }

        // Ask for the session's current state too, in case mitos-gui
        // is starting (or restarting after a crash) while the session
        // is already locked -- the reply comes back through the same
        // channel as everything else, handled in `poll_session_ipc`.
        if let Err(err) = write_message(&mut stream, &Request::SessionStatus { session_id }) {
            tracing::warn!("MITOS GUI: could not query session status: {err}");
        }

        let write_half = match stream.try_clone() {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!("MITOS GUI: could not clone mitos-session socket: {err}");
                return None;
            }
        };

        let (tx, rx) = mpsc::channel();
        let mut read_half = stream;
        std::thread::spawn(move || loop {
            match read_message::<_, Message>(&mut read_half) {
                Ok(msg) => {
                    if tx.send(msg).is_err() {
                        break; // main thread is gone
                    }
                }
                Err(err) => {
                    tracing::warn!("MITOS GUI: lost connection to mitos-session: {err}");
                    break;
                }
            }
        });

        tracing::info!("MITOS GUI: registered with mitos-session for session {session_id}");

        Some(Self { write_half, rx, session_id })
    }

    /// Send a request. Fire-and-forget from the caller's point of view
    /// -- the reply (a `Message::Response`) arrives later through
    /// `rx`, same as unsolicited `Event`s, since this connection is
    /// long-lived rather than one-shot request/reply.
    pub fn send(&mut self, request: &Request) {
        if let Err(err) = write_message(&mut self.write_half, request) {
            tracing::warn!("MITOS GUI: failed to send {request:?} to mitos-session: {err}");
        }
    }
}

/// Convenience used by `poll_session_ipc`: pulls the `locked` flag out
/// of a `SessionStatus` reply without the caller needing to match on
/// `Response` variants it doesn't care about.
pub fn session_locked(response: &Response) -> Option<bool> {
    match response {
        Response::Session(info) => Some(info.locked),
        _ => None,
    }
}
