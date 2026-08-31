//! Stage 6: Session management.
//!
//! Sends POSIX signals to PID 1 (mitos-init) to trigger
//! system state changes, exactly as documented in ASSEMBLY.md.

use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;

/// Reboot the system (SIGINT to PID 1).
pub fn reboot() {
    let _ = kill(Pid::from_raw(1), Signal::SIGINT);
}

/// Power off the system (SIGTERM to PID 1).
pub fn poweroff() {
    let _ = kill(Pid::from_raw(1), Signal::SIGTERM);
}

/// Halt the system (SIGQUIT to PID 1).
pub fn halt() {
    let _ = kill(Pid::from_raw(1), Signal::SIGQUIT);
}
