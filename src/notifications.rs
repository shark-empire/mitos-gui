//! Stage 6: Notification Engine.
//!
//! Manages transient desktop notifications. Currently driven internally;
//! will be wired to D-Bus (org.freedesktop.Notifications) in a future stage.

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
        let title_tex = self.text_renderer.render(title, 16.0, (255, 255, 255, 255))
            .and_then(TextTexture::from_rgba);
        let body_tex = self.text_renderer.render(body, 14.0, (200, 200, 200, 255))
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
    }

    /// Manually dismiss a notification (e.g., user clicks it).
    pub fn dismiss(&mut self, id: u32) {
        if let Some(pos) = self.active.iter().position(|n| n.id == id) {
            let n = self.active.remove(pos);
            self.history.push(n);
        }
    }

    /// Called every frame to remove expired notifications.
    /// Returns true if any notifications were removed (triggers redraw).
    pub fn tick(&mut self) -> bool {
        let now = Instant::now();
        let before = self.active.len();
        self.active.retain(|n| now.duration_since(n.created_at).as_secs() < n.duration_secs);
        self.active.len() != before
    }
}
