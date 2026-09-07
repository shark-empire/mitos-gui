//! Stage 6: Secure Authentication Prompt.
//!
//! Drawn directly by the compositor to prevent spoofing. Doubles as
//! mitos-session's lock screen (`is_lock_screen == true`): that variant
//! is driven entirely by `session_ipc`'s `ShowLockScreen`/`HideLockScreen`/
//! `AuthFeedback` events rather than anything decided locally -- see
//! mitos-session's docs/architecture.md. The other (privileged-action)
//! variant is still a local mock; wiring it to a real polkit-style
//! daemon is future work, tracked separately from the lock screen.

pub struct AuthPrompt {
    pub active: bool,
    pub app_name: String,
    pub action: String,
    pub password: String,
    pub error_msg: Option<String>,
    /// True when this prompt is mitos-session's screen lock rather than
    /// a one-off privileged-action prompt. Changes two behaviors:
    /// `cancel()` becomes a no-op (a lock screen can't be dismissed by
    /// pressing Escape), and submitting sends a real `Unlock` request
    /// over IPC instead of checking the password locally.
    pub is_lock_screen: bool,
    /// True from the moment a lock-screen submission sends `Unlock`
    /// until `AuthFeedback` comes back, so a slow PAM check can't be
    /// raced by mashing Enter.
    pub pending: bool,
}

impl AuthPrompt {
    pub fn new() -> Self {
        Self {
            active: false,
            app_name: String::new(),
            action: String::new(),
            password: String::new(),
            error_msg: None,
            is_lock_screen: false,
            pending: false,
        }
    }

    pub fn request(&mut self, app_name: &str, action: &str) {
        self.active = true;
        self.app_name = app_name.to_string();
        self.action = action.to_string();
        self.password.clear();
        self.error_msg = None;
        self.is_lock_screen = false;
        self.pending = false;
    }

    /// Enter mitos-session's screen-lock state. `message` is whatever
    /// mitos-session's `LockReason` translates to ("Locked", "Locked
    /// (idle)", ...) -- this module doesn't interpret it.
    pub fn lock(&mut self, message: &str) {
        self.active = true;
        self.app_name = "Locked".to_string();
        self.action = message.to_string();
        self.password.clear();
        self.error_msg = None;
        self.is_lock_screen = true;
        self.pending = false;
    }

    /// Authoritative dismissal, called only when mitos-session sends
    /// `HideLockScreen` (for a lock screen) -- never inferred locally
    /// from a password that merely looks right.
    pub fn hide(&mut self) {
        self.active = false;
        self.password.clear();
        self.error_msg = None;
        self.is_lock_screen = false;
        self.pending = false;
    }

    pub fn cancel(&mut self) {
        // A real lock screen can't be dismissed by the person sitting
        // at the keyboard just pressing Escape -- only mitos-session's
        // `HideLockScreen` (via `hide()`) can close it.
        if self.is_lock_screen {
            return;
        }
        self.active = false;
        self.password.clear();
    }

    /// Local mock check for the privileged-action variant only. Lock
    /// screen submissions are handled separately in `keyboard.rs`,
    /// which sends a real `Unlock` request instead of calling this.
    pub fn submit(&mut self) -> bool {
        // Mock authentication for now.
        // In production, this sends the password to the auth daemon.
        let success = self.password == "mitos"; 
        
        if success {
            self.active = false;
            self.password.clear();
            true
        } else {
            self.error_msg = Some("Incorrect password".to_string());
            self.password.clear();
            false
        }
    }
}
