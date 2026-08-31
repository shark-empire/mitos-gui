//! Stage 6: Secure Authentication Prompt.
//!
//! Drawn directly by the compositor to prevent spoofing.
//! Future: wired to polkitd or a custom mitos-auth daemon via IPC.

pub struct AuthPrompt {
    pub active: bool,
    pub app_name: String,
    pub action: String,
    pub password: String,
    pub error_msg: Option<String>,
}

impl AuthPrompt {
    pub fn new() -> Self {
        Self {
            active: false,
            app_name: String::new(),
            action: String::new(),
            password: String::new(),
            error_msg: None,
        }
    }

    pub fn request(&mut self, app_name: &str, action: &str) {
        self.active = true;
        self.app_name = app_name.to_string();
        self.action = action.to_string();
        self.password.clear();
        self.error_msg = None;
    }

    pub fn cancel(&mut self) {
        self.active = false;
        self.password.clear();
    }

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
