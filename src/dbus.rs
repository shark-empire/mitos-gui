//! D-Bus Service for org.freedesktop.Notifications using zbus (Pure Rust)
//!
//! Runs in a background thread and forwards notifications to the main compositor thread via an mpsc channel.

use zbus::blocking::Connection;
use zbus::interface;
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::Mutex;
use std::collections::HashMap;

// Shared state for the D-Bus interface
struct NotificationService {
    tx: mpsc::Sender<(String, String, String)>,
    next_id: Mutex<u32>,
}

#[interface(name = "org.freedesktop.Notifications")]
impl NotificationService {
    fn notify(
        &self,
        app_name: String,
        _replaces_id: u32,
        _app_icon: String,
        summary: String,
        body: String,
        _actions: Vec<String>,
        _hints: HashMap<String, zbus::zvariant::Value<'_>>,
        _expire_timeout: i32,
    ) -> u32 {
        let _ = self.tx.send((app_name, summary, body));
        let mut id = self.next_id.lock().unwrap();
        *id += 1;
        *id
    }

    fn close_notification(&self, _id: u32) {}

    fn get_server_information(&self) -> (String, String, String, String) {
        (
            "MITOS".to_string(),
            "MITOS".to_string(),
            "0.1.0".to_string(),
            "1.2".to_string(),
        )
    }

    fn get_capabilities(&self) -> Vec<String> {
        vec!["body".to_string()]
    }
}

pub struct DbusService {
    pub conn: Arc<Connection>,
    pub rx: mpsc::Receiver<(String, String, String)>,
}

impl DbusService {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let conn = Connection::session()?;
        let (tx, rx) = mpsc::channel();

        let service = NotificationService {
            tx,
            next_id: Mutex::new(0),
        };

        // Register the object on the D-Bus
        conn.object_server().at("/org/freedesktop/Notifications", service)?;

        // Request the well-known name
        conn.request_name("org.freedesktop.Notifications")?;

        // zbus::blocking::Connection automatically spawns a background thread 
        // to process incoming D-Bus messages and dispatch them to the interface.

        Ok(Self {
            conn: Arc::new(conn),
            rx,
        })
    }
}
