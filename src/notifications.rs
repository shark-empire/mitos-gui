//! Stage 6: Notification Engine.
//!
//! Manages transient desktop notifications. mitos-gui owns
//! org.freedesktop.Notifications (see dbus.rs), so toasts are pushed
//! directly into this manager rather than sent over D-Bus.

use std::time::Instant;
use crate::text::{TextRenderer, TextTexture};

#[derive(Clone, Debug)]
pub struct Notification {
    pub id: u32,
    pub app_name: String,
    pub title: String,
    pub body: String,
    pub created_at: Instant,
    pub duration_secs: u64,
    
    // Pre-rasterized text textures for GPU rendering
    pub title_tex: Option<TextTexture>,
    pub body_tex: Option<TextTexture>,
}

/// Toasts visible on screen at once. Past this, `push` dismisses the
/// oldest to make room -- otherwise a burst of notifications (a noisy
/// app, or several D-Bus clients firing close together) could stack up
/// and cover the whole screen.
const MAX_VISIBLE: usize = 4;

/// Dismissed/expired notifications kept for a future Notification
/// Center. Bounded so a long uptime can't grow this without limit.
const MAX_HISTORY: usize = 50;

pub struct NotificationManager {
    pub active: Vec<Notification>,
    pub history: Vec<Notification>, // For future Notification Center
    next_id: u32,
    text_renderer: TextRenderer,
}

impl NotificationManager {
    pub fn new() -> Self {
        Self {
            active: Vec::new(),
            history: Vec::new(),
            next_id: 1,
            text_renderer: TextRenderer::new(),
        }
    }

    /// Push a new notification to the screen.
    pub fn push(&mut self, app_name: &str, title: &str, body: &str) {
        let title_tex = self.text_renderer
            .render(title, 16.0, crate::theme::MitosTheme::TEXT.to_u8())
            .and_then(TextTexture::from_rgba);
        let body_tex = self.text_renderer
            .render(body, 14.0, crate::theme::MitosTheme::effective_text_muted().to_u8())
            .and_then(TextTexture::from_rgba);

        let notif = Notification {
            id: self.next_id,
            app_name: app_name.to_string(),
            title: title.to_string(),
            body: body.to_string(),
            created_at: Instant::now(),
            duration_secs: 5, // Auto-dismiss after 5 seconds
            title_tex,
            body_tex,
        };
        
        self.next_id += 1;
        self.active.push(notif);

        // Keep at most MAX_VISIBLE toasts on screen -- dismiss (and
        // archive) the oldest to make room for this one.
        if self.active.len() > MAX_VISIBLE {
            if let Some(oldest_id) = self.active.first().map(|n| n.id) {
                self.dismiss(oldest_id);
            }
        }
    }

    /// Manually dismiss a notification (e.g., user clicks it).
    pub fn dismiss(&mut self, id: u32) {
        if let Some(pos) = self.active.iter().position(|n| n.id == id) {
            let n = self.active.remove(pos);
            self.archive(n);
        }
    }

    /// Move a notification into bounded history -- whether it got there
    /// by manual dismissal, capacity eviction, or expiring on its own.
    fn archive(&mut self, n: Notification) {
        tracing::debug!(
            "MITOS GUI: notification archived: [{}] {} - {}",
            n.app_name, n.title, n.body,
        );

        self.history.push(n);

        if self.history.len() > MAX_HISTORY {
            self.history.remove(0);
        }
    }

    /// Called every frame to remove expired notifications.
    /// Returns true if any notifications were removed (triggers redraw).
    pub fn tick(&mut self) -> bool {
        let now = Instant::now();
        let before = self.active.len();

        let mut i = 0;
        while i < self.active.len() {
            if now.duration_since(self.active[i].created_at).as_secs() < self.active[i].duration_secs {
                i += 1;
            } else {
                let n = self.active.remove(i);
                self.archive(n);
            }
        }

        self.active.len() != before
    }
}
