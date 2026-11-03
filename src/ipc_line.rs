//! Shared plumbing for the daemon clients (`power_ipc`, `input_ipc`,
//! `bluetooth_ipc`): a tiny, non-blocking, newline-delimited Unix-socket
//! transport.
//!
//! Why not a reader thread + `mpsc` per connection, the way `dbus.rs` and
//! `session_ipc.rs` do it? Every reader thread is a stack and a kernel task
//! that exists for the whole session just to sit in `read()`. These daemons
//! talk rarely (a battery update now and then, a hot-plug, a pairing
//! request), so the main loop reads them itself, once per frame, with
//! non-blocking sockets:
//!
//! * no extra threads, no channels, no extra wake-ups;
//! * an idle, connected daemon costs one `read()` returning `EAGAIN` per
//!   frame, and a daemon that isn't running costs one failed `connect()`
//!   every few seconds at most (see [`Backoff`]);
//! * every buffer is bounded, so a misbehaving peer can't grow our memory.
//!
//! Nothing here knows any protocol -- callers split lines themselves.

use std::io::{self, Read, Write};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

/// Longest single line accepted from a peer. A peer that sends more than
/// this without a newline is treated as broken and dropped.
const MAX_LINE: usize = 256 * 1024;

/// Most bytes queued for a peer that isn't reading. Requests here are tens
/// of bytes, so reaching this means the peer is stuck.
const MAX_OUT: usize = 64 * 1024;

const READ_CHUNK: usize = 4096;

/// Most bytes read from one connection per call, so a chatty peer can't eat
/// a whole frame's budget -- whatever is left is picked up next frame.
const READ_BUDGET: usize = 64 * 1024;

/// A connected, non-blocking, line-oriented Unix stream socket.
pub struct LineConn {
    stream: UnixStream,
    rbuf: Vec<u8>,
    /// Start of the unread part of `rbuf`.
    rpos: usize,
    wbuf: Vec<u8>,
    closed: bool,
}

impl LineConn {
    /// Connects to `path`, then switches to non-blocking mode.
    ///
    /// The server must be running as root or as the same user as us
    /// (checked with `SO_PEERCRED`). Several of these sockets live in
    /// world-writable directories or are created by whoever starts the
    /// daemon first, and what comes back over them ends up on screen
    /// (battery state, pairing prompts), so a socket served by some other
    /// user is refused rather than trusted.
    pub fn connect(path: &str) -> io::Result<Self> {
        let stream = UnixStream::connect(path)?;

        match peer_uid(&stream) {
            Some(uid) if uid == 0 || uid == our_uid() => {}
            Some(uid) => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("{path} is served by uid {uid}, which is neither root nor us"),
                ));
            }
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("cannot verify who serves {path}"),
                ));
            }
        }

        Self::from_stream(stream)
    }

    /// Wraps an already-connected stream (non-blocking from here on). Also
    /// what the clients' unit tests use, with `UnixStream::pair()`.
    pub(crate) fn from_stream(stream: UnixStream) -> io::Result<Self> {
        stream.set_nonblocking(true)?;

        Ok(Self {
            stream,
            rbuf: Vec::new(),
            rpos: 0,
            wbuf: Vec::new(),
            closed: false,
        })
    }

    /// Reads whatever is available (bounded per call) into the line buffer.
    ///
    /// Returns `false` once the peer has closed the connection or
    /// misbehaved. Lines that were received before that point are still
    /// available from [`next_line`](Self::next_line), so callers should
    /// drain those *before* dropping the connection.
    pub fn fill(&mut self) -> bool {
        if self.closed {
            return false;
        }

        let mut chunk = [0u8; READ_CHUNK];
        let mut budget = READ_BUDGET;

        while budget > 0 {
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    self.closed = true;
                    break;
                }
                Ok(n) => {
                    self.rbuf.extend_from_slice(&chunk[..n]);
                    budget = budget.saturating_sub(n);

                    // Only look for a newline once the unread part is
                    // already over the limit -- the common case never scans.
                    if self.rbuf.len() - self.rpos > MAX_LINE
                        && !self.rbuf[self.rpos..].contains(&b'\n')
                    {
                        self.closed = true;
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    self.closed = true;
                    break;
                }
            }
        }

        !self.closed
    }

    /// Pops the next complete line (without its line ending) into `out`.
    /// Returns `false` when no complete line is buffered.
    pub fn next_line(&mut self, out: &mut String) -> bool {
        let start = self.rpos;

        let Some(rel) = self.rbuf[start..].iter().position(|&b| b == b'\n') else {
            return false;
        };

        let end = start + rel;

        let mut line = &self.rbuf[start..end];
        if line.last() == Some(&b'\r') {
            line = &line[..line.len() - 1];
        }

        out.clear();
        out.push_str(&String::from_utf8_lossy(line));

        self.rpos = end + 1;

        if self.rpos == self.rbuf.len() {
            // Everything consumed: start over, and give back the memory
            // if one unusually large reply ballooned the buffer.
            self.rpos = 0;
            if self.rbuf.capacity() > 16 * 1024 {
                self.rbuf = Vec::new();
            } else {
                self.rbuf.clear();
            }
        } else if self.rpos > 32 * 1024 {
            // A long backlog: compact so the consumed prefix doesn't pile up.
            let consumed = self.rpos;
            self.rbuf.drain(..consumed);
            self.rpos = 0;
        }

        true
    }

    /// Queues `bytes` and writes as much as the socket takes right now.
    /// Returns `false` if the peer is gone, or has stopped reading for so
    /// long that its queue hit the cap.
    pub fn send(&mut self, bytes: &[u8]) -> bool {
        if self.closed || self.wbuf.len() + bytes.len() > MAX_OUT {
            return false;
        }
        self.wbuf.extend_from_slice(bytes);
        self.flush()
    }

    /// Writes queued output without blocking. `false` = the peer is gone.
    pub fn flush(&mut self) -> bool {
        while !self.wbuf.is_empty() {
            match self.stream.write(&self.wbuf) {
                Ok(0) => return false,
                Ok(n) => {
                    self.wbuf.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return false,
            }
        }
        true
    }

    pub fn has_pending_output(&self) -> bool {
        !self.wbuf.is_empty()
    }
}

/// uid of the process serving the other end of `stream`.
fn peer_uid(stream: &UnixStream) -> Option<u32> {
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;

    // SAFETY: `cred` and `len` are valid for writes of the sizes passed, and
    // the fd is a live socket owned by `stream` for the whole call.
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut libc::ucred as *mut libc::c_void,
            &mut len,
        )
    };

    if rc == 0 {
        Some(cred.uid)
    } else {
        None
    }
}

fn our_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// Retry schedule for a daemon that isn't (yet) there: 1s, 2s, 4s ... capped
/// at 30s, so a machine without Bluetooth never spins on a missing socket.
pub struct Backoff {
    next_try: Instant,
    delay: Duration,
}

impl Backoff {
    pub fn new() -> Self {
        Self {
            next_try: Instant::now(),
            delay: Duration::from_secs(1),
        }
    }

    /// Whether it's time for another connection attempt.
    pub fn due(&self) -> bool {
        Instant::now() >= self.next_try
    }

    pub fn failed(&mut self) {
        self.next_try = Instant::now() + self.delay;
        self.delay = (self.delay * 2).min(Duration::from_secs(30));
    }

    pub fn succeeded(&mut self) {
        self.delay = Duration::from_secs(1);
        self.next_try = Instant::now();
    }

    /// An established connection dropped: pause briefly before reconnecting
    /// (a restarting daemon needs a moment to bind its socket again).
    pub fn lost(&mut self) {
        self.delay = Duration::from_secs(2);
        self.next_try = Instant::now() + Duration::from_secs(1);
    }
}

/// Logs a failed connection attempt once per outage instead of on every
/// retry. A missing daemon is normal (debug); a socket we may not use, or
/// that someone else serves, deserves a warning.
pub fn log_connect_failure(what: &str, path: &str, err: &io::Error, already_logged: &mut bool) {
    if *already_logged {
        return;
    }
    *already_logged = true;

    match err.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => {
            tracing::debug!("MITOS GUI: {what} not available at {path}: {err}");
        }
        _ => {
            tracing::warn!("MITOS GUI: cannot use {what} at {path}: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (LineConn, UnixStream) {
        let (a, b) = UnixStream::pair().unwrap();
        a.set_nonblocking(true).unwrap();
        let conn = LineConn {
            stream: a,
            rbuf: Vec::new(),
            rpos: 0,
            wbuf: Vec::new(),
            closed: false,
        };
        (conn, b)
    }

    #[test]
    fn splits_lines_across_reads_and_strips_cr() {
        let (mut conn, mut peer) = pair();
        let mut line = String::new();

        peer.write_all(b"one\r\ntw").unwrap();
        assert!(conn.fill());
        assert!(conn.next_line(&mut line));
        assert_eq!(line, "one");
        assert!(!conn.next_line(&mut line)); // "tw" isn't complete yet

        peer.write_all(b"o\nthree\n").unwrap();
        assert!(conn.fill());
        assert!(conn.next_line(&mut line));
        assert_eq!(line, "two");
        assert!(conn.next_line(&mut line));
        assert_eq!(line, "three");
        assert!(!conn.next_line(&mut line));
    }

    #[test]
    fn lines_received_before_eof_are_still_readable() {
        let (mut conn, mut peer) = pair();
        peer.write_all(b"last words\n").unwrap();
        drop(peer);

        assert!(!conn.fill()); // closed...
        let mut line = String::new();
        assert!(conn.next_line(&mut line)); // ...but nothing was lost
        assert_eq!(line, "last words");
        assert!(!conn.fill());
    }

    #[test]
    fn send_reaches_the_peer() {
        let (mut conn, mut peer) = pair();
        assert!(conn.send(b"hello\n"));
        assert!(!conn.has_pending_output());

        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut buf = [0u8; 16];
        let n = peer.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"hello\n");
    }

    #[test]
    fn oversized_line_is_refused() {
        let (mut conn, mut peer) = pair();
        peer.set_nonblocking(true).unwrap();

        // 200 x 8 KiB is far more than MAX_LINE; the connection reads up to
        // 64 KiB per fill, so it must give up long before the loop ends.
        let junk = vec![b'x'; 8192];
        let mut dropped = false;
        for _ in 0..200 {
            let _ = peer.write(&junk); // WouldBlock when the buffer is full: fine
            if !conn.fill() {
                dropped = true;
                break;
            }
        }
        assert!(dropped);
        assert!(!conn.fill());
    }

    #[test]
    fn backoff_doubles_up_to_the_cap_and_resets() {
        let mut b = Backoff::new();
        assert!(b.due());

        for _ in 0..10 {
            b.failed();
        }
        assert_eq!(b.delay, Duration::from_secs(30));
        assert!(!b.due());

        b.succeeded();
        assert!(b.due());
        assert_eq!(b.delay, Duration::from_secs(1));
    }
}
