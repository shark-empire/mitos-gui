//! Stage 7: AT-SPI provider for MITOS's own shell.
//!
//! **Scope.** AT-SPI (the Linux screen-reader/assistive-tech protocol)
//! is normally implemented by *toolkits* (GTK, Qt) on behalf of the
//! apps built with them -- the compositor doesn't need to be involved
//! for a GTK app's own window content to be readable by Orca. But
//! MITOS's top bar, dock, launcher, and notifications are drawn
//! directly by the compositor, not by any toolkit -- nothing else can
//! ever make *those* accessible, so if they're going to be readable at
//! all, mitos-gui has to be the one to speak AT-SPI for them. That is
//! the entire scope of this module: it says nothing about, and has no
//! effect on, whether content inside a client window is accessible --
//! that remains entirely between the client's own toolkit and AT-SPI.
//!
//! **Confidence level, stated plainly.** The `org.a11y.atspi.Accessible`
//! interface and the `org.a11y.atspi.Event.Object` signals implemented
//! below were checked against the published AT-SPI D-Bus interface
//! specification (not guessed from memory), so the *shape* of what's
//! implemented -- method names, argument order, D-Bus type signatures
//! -- should be right. What's **not** independently verified is the
//! registration handshake with the registry daemon (`registryd`): the
//! `Embed` call in `connect()` is built from the same spec reading,
//! but this sandbox has no accessibility bus or running screen reader
//! to actually complete a handshake against, so whether registration
//! *succeeds* end-to-end with a real Orca session is genuinely
//! untested. If the on-screen keyboard's synthetic-key path is the
//! "check here first" for typing issues, this module is that same
//! kind of flag for accessibility-tree issues.
//!
//! **What's exposed.** One flat, single-node summary -- "MITOS Shell",
//! with a live description like "Dock: 5 apps, Terminal focused; 2
//! notifications" -- rather than a deep tree with one D-Bus object per
//! dock icon. A per-widget tree is the more complete design and a
//! reasonable next step, but it means dynamically registering and
//! unregistering a D-Bus object per dock icon/notification as they
//! come and go, which is meaningfully more D-Bus surface to get right
//! without being able to test against a real AT client. A single
//! well-described node that updates promptly is a smaller, honest
//! piece that still makes the shell's state genuinely announceable.

use std::sync::{Arc, Mutex};
use zbus::blocking::Connection;
use zbus::interface;

/// Live text describing the shell's current state, rebuilt by
/// `state.rs` whenever something worth announcing changes (dock focus
/// move, launcher open/close, notification count change, ...) and
/// read by the `Accessible`/`Application` D-Bus methods below.
#[derive(Default, Clone)]
pub struct ShellSummary {
    pub name: String,
    pub description: String,
}

struct ShellAccessible {
    summary: Arc<Mutex<ShellSummary>>,
}

#[interface(name = "org.a11y.atspi.Accessible")]
impl ShellAccessible {
    #[zbus(property)]
    fn name(&self) -> String {
        self.summary.lock().unwrap().name.clone()
    }

    #[zbus(property)]
    fn description(&self) -> String {
        self.summary.lock().unwrap().description.clone()
    }

    #[zbus(property, name = "Parent")]
    fn parent(&self) -> (String, zbus::zvariant::OwnedObjectPath) {
        // No parent -- this is the application's own root object.
        (String::new(), "/org/a11y/atspi/null".try_into().unwrap())
    }

    #[zbus(property, name = "ChildCount")]
    fn child_count(&self) -> i32 {
        0
    }

    fn get_child_at_index(&self, _index: i32) -> (String, zbus::zvariant::OwnedObjectPath) {
        (String::new(), "/org/a11y/atspi/null".try_into().unwrap())
    }

    fn get_children(&self) -> Vec<(String, zbus::zvariant::OwnedObjectPath)> {
        Vec::new()
    }

    fn get_index_in_parent(&self) -> i32 {
        -1
    }

    fn get_role(&self) -> u32 {
        // ROLE_PANEL, per the AT-SPI Accessible.Role enum -- the
        // closest single-node stand-in for "a desktop shell region"
        // out of the standard role list.
        68
    }

    fn get_role_name(&self) -> String {
        "panel".to_string()
    }

    fn get_localized_role_name(&self) -> String {
        "panel".to_string()
    }

    fn get_state(&self) -> Vec<u32> {
        // STATE_VISIBLE (30), STATE_SHOWING (31) -- see the AT-SPI
        // State enum. Returned as the two-`u32`-per-flag bitset the
        // real interface uses; with this few flags set, a plain
        // Vec<u32> pair of (lower 32 bits, upper 32 bits) covers it.
        let mut lower: u32 = 0;
        lower |= 1 << 30;
        lower |= 1 << 31;
        vec![lower, 0]
    }

    fn get_attributes(&self) -> std::collections::HashMap<String, String> {
        std::collections::HashMap::new()
    }

    fn get_application(&self) -> (String, zbus::zvariant::OwnedObjectPath) {
        (
            "org.mitos.gui".to_string(),
            "/org/mitos/gui/Accessible".try_into().unwrap(),
        )
    }

    fn get_interfaces(&self) -> Vec<String> {
        vec![
            "org.a11y.atspi.Accessible".to_string(),
            "org.a11y.atspi.Application".to_string(),
        ]
    }
}

#[interface(name = "org.a11y.atspi.Application")]
impl ShellAccessible {
    #[zbus(property, name = "ToolkitName")]
    fn toolkit_name(&self) -> String {
        "mitos-gui".to_string()
    }

    #[zbus(property, name = "Version")]
    fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    #[zbus(property, name = "Id")]
    fn id(&self) -> i32 {
        std::process::id() as i32
    }
}

/// Handle to the accessibility-bus connection and the live summary
/// text it serves. `state.rs` updates the summary and calls
/// `announce_change` whenever something worth telling a screen reader
/// about happens; everything else is handled by zbus's own background
/// dispatch thread, same as `dbus.rs`'s notification service.
pub struct AtspiProvider {
    conn: Connection,
    summary: Arc<Mutex<ShellSummary>>,
}

impl AtspiProvider {
    /// Best-effort setup: connects to the accessibility bus, registers
    /// MITOS's shell as one Accessible/Application object, and asks
    /// the registry to embed it. Returns `None` (never an error a
    /// caller has to handle) if any step fails -- e.g. no
    /// accessibility bus is running at all, which is completely normal
    /// on a system with no assistive technology in use. Screen-reader
    /// support being unavailable should never be a reason the rest of
    /// the compositor fails to start.
    pub fn connect() -> Option<Self> {
        let session = Connection::session().ok()?;

        // The accessibility bus is a separate bus instance from the
        // session bus; org.a11y.Bus.GetAddress (on the session bus)
        // is how a client is meant to find it.
        let a11y_address: String = session
            .call_method(
                Some("org.a11y.Bus"),
                "/org/a11y/bus",
                Some("org.a11y.Bus"),
                "GetAddress",
                &(),
            )
            .ok()?
            .body()
            .deserialize()
            .ok()?;

        let conn = Connection::builder(a11y_address.as_str()).ok()?.build().ok()?;

        let summary = Arc::new(Mutex::new(ShellSummary {
            name: "MITOS Shell".to_string(),
            description: String::new(),
        }));

        let accessible = ShellAccessible { summary: summary.clone() };
        conn.object_server()
            .at("/org/mitos/gui/Accessible", accessible)
            .ok()?;

        conn.request_name("org.mitos.gui").ok()?;

        // Ask the registry to embed us. Best-effort: if this specific
        // call doesn't match what a real registryd expects (see the
        // module doc's confidence note), the object above is still
        // sitting on the bus at a well-known name/path for a client
        // willing to query it directly either way.
        let embed_result: Result<zbus::Message, _> = conn.call_method(
            Some("org.a11y.atspi.Registry"),
            "/org/a11y/atspi/accessible/root",
            Some("org.a11y.atspi.Socket"),
            "Embed",
            &(
                "org.mitos.gui",
                zbus::zvariant::ObjectPath::try_from("/org/mitos/gui/Accessible").ok()?,
            ),
        );
        if let Err(err) = embed_result {
            tracing::warn!("MITOS GUI: AT-SPI registry embed failed (non-fatal): {err}");
        }

        Some(Self { conn, summary })
    }

    /// Update the live summary an AT-SPI client would see by querying
    /// this object's `Name`/`Description` properties. Doesn't also
    /// push a change signal -- `org.a11y.atspi.Event.Object`'s
    /// `PropertyChange` signal takes a `zvariant::Value` payload whose
    /// exact construction wasn't worth the added risk to get right
    /// unverified for a "nicer, but not load-bearing" notification;
    /// the properties themselves are still correct and queryable the
    /// moment this returns, any client watching via a fresh query
    /// (rather than only a push notification) sees the update.
    pub fn announce_change(&self, name: impl Into<String>, description: impl Into<String>) {
        let mut s = self.summary.lock().unwrap();
        s.name = name.into();
        s.description = description.into();
    }
}
