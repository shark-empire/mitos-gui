//! Stage 6: Secure Authentication Prompt.
//!
//! Drawn directly by the compositor to prevent spoofing -- the whole
//! reason this lives here instead of as a regular Wayland client is
//! that no client-drawn surface can ever be trusted not to be a
//! lookalike (see mitos-session's docs/security.md). Covers three
//! things that all share this one modal "type your password, or
//! don't" surface:
//!
//! - **Lock screen** (`is_lock_screen == true`): driven entirely by
//!   `session_ipc`'s `ShowLockScreen`/`HideLockScreen`/`AuthFeedback`
//!   events, never decided locally. Can't be dismissed with Escape.
//! - **Elevation prompt** (`request_id.is_some()`): mitos-session
//!   asking, on mitos-service's behalf, to verify the user before a
//!   privileged action proceeds -- driven by `ShowElevationPrompt`/
//!   `ElevationFeedback`/`HideElevationPrompt`. *Can* be cancelled.
//!   Multiple can be pending across the whole desktop at once (see
//!   mitos-session's `[elevation].max_pending_per_session`); this
//!   struct shows one at a time and queues the rest in `queue`,
//!   oldest first, showing the next once the current one closes.
//! - **Local mock** (`request_id.is_none() && !is_lock_screen`):
//!   the pre-existing `Super+Shift+A` dev trigger, checked against a
//!   hardcoded password in `submit()` rather than talking to
//!   mitos-session at all. Left in place for exercising the prompt's
//!   rendering without a live mitos-session + mitos-service to hand;
//!   real elevation prompts never go through `submit()`.

use crate::text::{TextRenderer, TextTexture};
use std::collections::VecDeque;

/// An elevation prompt that arrived while another one (or the lock
/// screen) was already showing -- shown once whatever's currently
/// active closes, oldest first.
struct QueuedElevation {
    request_id: u64,
    app_name: String,
    subtitle: String,
    critical: bool,
}

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
    /// True from the moment a submission sends `Unlock` or
    /// `RespondElevation` until the matching feedback comes back, so a
    /// slow PAM check can't be raced by mashing Enter (or Escape).
    pub pending: bool,
    /// Set only for a real elevation prompt -- which of mitos-session's
    /// pending requests this is, so submitting or cancelling references
    /// the right one. `None` for the lock screen and for the local mock
    /// prompt, neither of which mitos-session is tracking by id.
    pub request_id: Option<u64>,
    /// Set alongside `request_id` when mitos-service flagged this
    /// action `ElevationRisk::Critical` -- purely a rendering hint
    /// (see `renderer::collect_auth_elements`) so a critical prompt
    /// reads as more urgent than a merely-elevated one. Like `action`,
    /// this is display-only: the actual risk classification is
    /// mitos-service's call, made before mitos-session ever asked.
    pub critical: bool,
    /// Cached rasterizations of `app_name`/`action`, refreshed only
    /// when those change (`request`/`lock`/`activate_elevation`) --
    /// unlike `error_msg`, which the compositor re-renders fresh every
    /// frame it's shown (see `collect_auth_elements` in renderer.rs),
    /// since a lock screen can legitimately sit on screen for hours
    /// and re-rasterizing two short, unchanging lines that whole time
    /// would be pure waste.
    pub title_tex: Option<TextTexture>,
    pub subtitle_tex: Option<TextTexture>,
    pub text_renderer: TextRenderer,
    queue: VecDeque<QueuedElevation>,
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
            request_id: None,
            critical: false,
            title_tex: None,
            subtitle_tex: None,
            text_renderer: TextRenderer::new(),
            queue: VecDeque::new(),
        }
    }

    fn refresh_title_textures(&mut self) {
        self.title_tex = self
            .text_renderer
            .render(&self.app_name, 18.0, (255, 255, 255, 255))
            .and_then(TextTexture::from_rgba);
        self.subtitle_tex = self
            .text_renderer
            .render(&self.action, 14.0, (210, 210, 210, 255))
            .and_then(TextTexture::from_rgba);
    }

    /// Local mock trigger (`Super+Shift+A`) -- not tied to any
    /// mitos-session request. See `submit()`.
    pub fn request(&mut self, app_name: &str, action: &str) {
        self.active = true;
        self.app_name = app_name.to_string();
        self.action = action.to_string();
        self.password.clear();
        self.error_msg = None;
        self.is_lock_screen = false;
        self.pending = false;
        self.request_id = None;
        self.critical = false;
        self.refresh_title_textures();
    }

    /// Enter mitos-session's screen-lock state. `message` is whatever
    /// mitos-session's `LockReason` translates to ("Locked", "Locked
    /// (idle)", ...) -- this module doesn't interpret it. The lock
    /// screen always wins the modal surface outright: it doesn't go
    /// through the elevation queue, and an elevation prompt that was
    /// active gets pushed back in front of it (`request_elevation`
    /// re-queues on top of a lock screen the same as it would on top
    /// of another prompt).
    pub fn lock(&mut self, message: &str) {
        self.active = true;
        self.app_name = crate::i18n::t("locked");
        self.action = message.to_string();
        self.password.clear();
        self.error_msg = None;
        self.is_lock_screen = true;
        self.pending = false;
        self.request_id = None;
        self.critical = false;
        self.refresh_title_textures();
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
        self.request_id = None;
        self.critical = false;
        self.title_tex = None;
        self.subtitle_tex = None;
        self.try_advance_queue();
    }

    pub fn cancel(&mut self) {
        // A real lock screen can't be dismissed by the person sitting
        // at the keyboard just pressing Escape -- only mitos-session's
        // `HideLockScreen` (via `hide()`) can close it. A real
        // elevation prompt has its own cancel path (`keyboard.rs`'s
        // `cancel_elevation`, which sends `RespondElevation` and waits
        // for mitos-session's own `HideElevationPrompt` rather than
        // closing locally) -- this bare `cancel()` is only ever called
        // for the local mock prompt now.
        if self.is_lock_screen || self.request_id.is_some() {
            return;
        }
        self.active = false;
        self.password.clear();
        self.title_tex = None;
        self.subtitle_tex = None;
    }

    /// Local mock check for the dev-trigger prompt only (`request_id
    /// == None && !is_lock_screen`). Lock-screen submissions are
    /// handled in `keyboard.rs::submit_lock_screen`, real elevation
    /// submissions in `keyboard.rs::submit_elevation` -- both send a
    /// real request over IPC instead of calling this.
    pub fn submit(&mut self) -> bool {
        // Mock authentication for now.
        // In production, this sends the password to the auth daemon.
        let success = self.password == "mitos";

        if success {
            self.active = false;
            self.password.clear();
            self.critical = false;
            self.title_tex = None;
            self.subtitle_tex = None;
            true
        } else {
            self.error_msg = Some(crate::i18n::t("incorrect_password"));
            self.password.clear();
            false
        }
    }

    /// `Event::ShowElevationPrompt` arrived. If nothing's currently
    /// occupying the modal surface, show it immediately; otherwise
    /// queue it behind whatever is (the lock screen, or another
    /// elevation prompt) -- `try_advance_queue` shows it once that
    /// clears.
    pub fn request_elevation(&mut self, request_id: u64, app_name: &str, subtitle: &str, critical: bool) {
        if self.active {
            self.queue.push_back(QueuedElevation {
                request_id,
                app_name: app_name.to_string(),
                subtitle: subtitle.to_string(),
                critical,
            });
            return;
        }
        self.activate_elevation(request_id, app_name, subtitle, critical);
    }

    fn activate_elevation(&mut self, request_id: u64, app_name: &str, subtitle: &str, critical: bool) {
        self.active = true;
        self.app_name = app_name.to_string();
        self.action = subtitle.to_string();
        self.password.clear();
        self.error_msg = None;
        self.is_lock_screen = false;
        self.pending = false;
        self.request_id = Some(request_id);
        self.critical = critical;
        self.refresh_title_textures();
    }

    /// The active elevation prompt is over (its `HideElevationPrompt`
    /// arrived, however it got resolved) -- close it and show whatever
    /// was next in line, if anything.
    pub fn resolve_elevation(&mut self) {
        self.active = false;
        self.password.clear();
        self.error_msg = None;
        self.pending = false;
        self.request_id = None;
        self.critical = false;
        self.title_tex = None;
        self.subtitle_tex = None;
        self.try_advance_queue();
    }

    /// A queued (not currently shown) elevation prompt was resolved by
    /// someone else before its turn came up -- e.g. mitos-session
    /// abandoned it (timeout, session ended) while it was still
    /// waiting in line here. No-op if `request_id` isn't queued
    /// (including if it's the *active* one -- that's `resolve_elevation`'s
    /// job, not this one's).
    pub fn remove_from_queue(&mut self, request_id: u64) {
        self.queue.retain(|q| q.request_id != request_id);
    }

    fn try_advance_queue(&mut self) {
        if self.active {
            return;
        }
        if let Some(next) = self.queue.pop_front() {
            self.activate_elevation(next.request_id, &next.app_name, &next.subtitle, next.critical);
        }
    }

    /// How many more elevation prompts are waiting behind whatever's
    /// currently shown (including behind the lock screen, since
    /// `request_elevation` queues rather than interrupting it too).
    /// Used only for the "+N more" hint in `collect_auth_elements`.
    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }
}
