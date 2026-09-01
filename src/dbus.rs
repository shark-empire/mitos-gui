//! D-Bus Service for org.freedesktop.Notifications
use dbus::blocking::SyncConnection;
use dbus_crossroads::{Crossroads, Context};
use std::sync::Arc;
use std::sync::mpsc;

pub struct DbusService {
    pub conn: Arc<SyncConnection>,
    pub rx: mpsc::Receiver<(String, String, String)>,
}

impl DbusService {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let conn = SyncConnection::new_session()?;
        conn.request_name("org.freedesktop.Notifications", false, true, false)?;

        let mut cr = Crossroads::new();
        let (tx, rx) = mpsc::channel();

        let iface_token = cr.register("org.freedesktop.Notifications", |b| {
            b.method(
                "Notify",
                ("app_name", "replaces_id", "app_icon", "summary", "body", "actions", "hints", "expire_timeout"),
                ("id",),
                move |_ctx: &mut Context, app_name: String, _replaces_id: u32, _icon: String, summary: String, body: String, _actions: Vec<String>, _hints: dbus::arg::PropMap, _timeout: i32| {
                    let _ = tx.send((app_name, summary, body));
                    Ok((1u32,))
                }
            );

            b.method("CloseNotification", ("id",), |_ctx: &mut Context, _id: u32| { Ok(()) });

            b.method(
                "GetServerInformation",
                (),
                ("name", "vendor", "version", "spec_version"),
                |_ctx: &mut Context| {
                    Ok((
                        "MITOS".to_string(),
                        "MITOS".to_string(),
                        "0.1.0".to_string(),
                        "1.2".to_string(),
                    ))
                },
            );

            b.method(
                "GetCapabilities",
                (),
                ("return_caps",),
                |_ctx: &mut Context| { Ok((vec!["body".to_string()],)) },
            );
        });

        cr.insert("/org/freedesktop/Notifications", &[iface_token], ());

        let conn_clone = conn.clone();
        std::thread::spawn(move || {
            let _ = cr.serve(&conn_clone);
        });

        Ok(Self {
            conn: Arc::new(conn),
            rx,
        })
    }
}
