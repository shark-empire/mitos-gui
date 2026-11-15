//! Wake-up plumbing: what lets the production loop *sleep* when idle.
//!
//! The DRM loop used to wake every 16 ms, forever, because several things
//! were noticed by polling rather than by readiness: Wayland client
//! requests, new client connections, the D-Bus and mitos-session reader
//! threads' channels, and (since `services.rs`) the daemon sockets. Idle,
//! that is ~60 wake-ups a second doing nothing, on a machine whose whole
//! point is to be light.
//!
//! Now each of those wakes the loop instead:
//!
//! * **Reader threads** (D-Bus, mitos-session) call [`wake`] after sending
//!   on their channel. It pings a `calloop` ping source ([`install_ping`]),
//!   which is just an `eventfd`.
//! * **Sockets** (the Wayland display and listening socket, the mitos-power /
//!   mitos-bluetooth / mitos-input connections) are added to a [`WakeSet`]:
//!   one `epoll` instance whose own fd turns readable whenever any watched
//!   fd does. That single fd is registered with `calloop` once, at startup
//!   ([`register_sockets`]), so connections that come and go (a daemon
//!   restarting) never touch the event loop -- they just add themselves to
//!   the set, and a closed socket drops out of it on its own.
//!
//! The wake-up callbacks do nothing. They exist only to end the loop's
//! `epoll_wait`; the existing per-iteration code (`dispatch_clients`,
//! `poll_dbus`, `poll_services`, ...) then does the actual reading, exactly
//! as it did when it ran on a timer.
//!
//! The winit dev backend isn't wired up (the host windowing system's
//! events are still polled), so it keeps its 16 ms tick.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use calloop::generic::Generic;
use calloop::ping::Ping;
use calloop::{Interest, LoopHandle, Mode, PostAction};

/// How long the production loop sleeps with nothing to do, once every
/// wake-up source above is registered. Anything that wakes the loop does
/// so immediately; this only bounds how late *time-driven* work -- the
/// clock's minute tick, toast expiry, the OSD timeout, reconnect attempts
/// -- can run. (Without the wake-up sources the loop keeps the old 16 ms.)
pub const IDLE_TIMEOUT: Duration = Duration::from_millis(250);

static PING: Mutex<Option<Ping>> = Mutex::new(None);

/// Wakes the main loop from any thread. A no-op until [`install_ping`] ran
/// (and always on the winit dev backend, which doesn't install one).
pub fn wake() {
    if let Ok(guard) = PING.lock() {
        if let Some(ping) = guard.as_ref() {
            ping.ping();
        }
    }
}

/// Registers the ping source with the loop and publishes its sender for
/// [`wake`]. Call it before spawning the threads that use `wake()`.
pub fn install_ping<'l, Data>(handle: &LoopHandle<'l, Data>) -> Result<(), String> {
    let (ping, source) = calloop::ping::make_ping().map_err(|e| e.to_string())?;

    handle
        .insert_source(source, |(), _, _| {})
        .map_err(|e| format!("{e:?}"))?;

    match PING.lock() {
        Ok(mut guard) => {
            // Keep one sender alive for the whole run: a ping source
            // removes itself once every sender is gone.
            *guard = Some(ping);
            Ok(())
        }
        Err(_) => Err("wake-up ping lock poisoned".to_string()),
    }
}

/// One `epoll` instance that aggregates the readiness of many fds into the
/// readiness of a single one (see the module docs).
#[derive(Clone)]
pub struct WakeSet {
    epfd: Arc<OwnedFd>,
}

impl WakeSet {
    pub fn new() -> io::Result<Self> {
        // SAFETY: a plain syscall with no pointer arguments.
        let fd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        // SAFETY: `fd` was just returned by the kernel and nothing else owns it.
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };

        Ok(Self {
            epfd: Arc::new(owned),
        })
    }

    /// Watches `fd` for input. Level-triggered, so a socket with unread
    /// data keeps the set readable until it's drained -- which the loop's
    /// normal polling does right after waking. A socket that's closed (or
    /// dropped) leaves the set by itself; adding one twice is harmless.
    pub fn add(&self, fd: RawFd) {
        let mut event = libc::epoll_event {
            events: libc::EPOLLIN as u32,
            u64: fd as u64,
        };

        // SAFETY: `event` is a valid epoll_event for the duration of the
        // call, and both fds are plain integers the kernel validates.
        let rc = unsafe {
            libc::epoll_ctl(
                self.epfd.as_raw_fd(),
                libc::EPOLL_CTL_ADD,
                fd,
                &mut event,
            )
        };

        if rc != 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() != Some(libc::EEXIST) {
                tracing::debug!("MITOS GUI: could not watch fd {fd} for wake-ups: {err}");
            }
        }
    }

    /// A second handle to the same epoll instance, for registering with
    /// the event loop (the loop owns its copy; this set keeps its own).
    pub fn source_fd(&self) -> io::Result<OwnedFd> {
        self.epfd.try_clone()
    }
}

/// Registers the wake-up for every socket in `set`, plus the two Wayland
/// fds (client requests on the display, new connections on the listening
/// socket). `Err` means the loop must keep polling -- the caller falls back
/// to its old short timeout.
pub fn register_sockets<'l, Data>(
    handle: &LoopHandle<'l, Data>,
    set: &WakeSet,
    display_fd: RawFd,
    listener_fd: RawFd,
) -> Result<(), String> {
    let source_fd = set.source_fd().map_err(|e| e.to_string())?;

    set.add(display_fd);
    set.add(listener_fd);

    handle
        .insert_source(
            Generic::new(source_fd, Interest::READ, Mode::Level),
            |_, _, _| Ok(PostAction::Continue),
        )
        .map_err(|e| format!("{e:?}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    fn readable(fd: RawFd) -> bool {
        let mut p = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `p` is a valid pollfd and we pass a count of exactly one.
        let n = unsafe { libc::poll(&mut p, 1, 0) };
        n > 0 && (p.revents & libc::POLLIN) != 0
    }

    #[test]
    fn the_set_turns_readable_when_a_watched_fd_does() {
        let set = WakeSet::new().unwrap();
        let source = set.source_fd().unwrap();
        let (a, mut b) = UnixStream::pair().unwrap();
        set.add(a.as_raw_fd());

        assert!(!readable(source.as_raw_fd()));

        b.write_all(b"x").unwrap();
        assert!(readable(source.as_raw_fd()));
    }

    #[test]
    fn a_closed_fd_leaves_the_set_by_itself() {
        let set = WakeSet::new().unwrap();
        let source = set.source_fd().unwrap();
        let (a, mut b) = UnixStream::pair().unwrap();
        set.add(a.as_raw_fd());

        b.write_all(b"x").unwrap();
        assert!(readable(source.as_raw_fd()));

        drop(a); // closing the watched fd removes it from the set
        assert!(!readable(source.as_raw_fd()));
    }

    #[test]
    fn watching_the_same_fd_twice_is_harmless() {
        let set = WakeSet::new().unwrap();
        let (a, mut b) = UnixStream::pair().unwrap();
        set.add(a.as_raw_fd());
        set.add(a.as_raw_fd());

        b.write_all(b"x").unwrap();
        assert!(readable(set.source_fd().unwrap().as_raw_fd()));
    }

    #[test]
    fn waking_before_install_is_a_no_op() {
        wake(); // must not panic or block
    }
}
