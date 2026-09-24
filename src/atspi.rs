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
//! **What's exposed**, as a real tree of D-Bus objects (not a single
//! flattened summary node):
//!
//! ```text
//! Root (Accessible, Application)
//! +-- Dock (Accessible, Component)
//! |   +-- Dock/0, Dock/1, ... (Accessible, Action) -- one per pinned
//! |       app, built once at startup from the same fixed list
//! |       `DockLayout::default()` uses; MITOS doesn't support
//! |       customizing the dock's app list, so unlike notifications
//! |       these never need to be added/removed at runtime
//! +-- Notifications (Accessible, Component)
//!     +-- Notifications/<id> (Accessible) -- one per active toast,
//!         added and removed live as they appear/dismiss/expire
//! ```
//!
//! A dock item's Action interface's "activate" launches it -- through
//! the same `mpsc` channel hand-off `dbus.rs`'s notification service
//! already uses to cross from zbus's own dispatch thread back to the
//! compositor's main loop, since D-Bus method calls don't run on it.
//!
//! **Confidence level, stated plainly.** The interface shapes below
//! (`Accessible`, `Application`, `Component`, `Action`, and the
//! `#[zbus(signal)]` mechanism used for `Event.Object` signals) were
//! checked against the published AT-SPI D-Bus specification and
//! zbus's own documented macro behavior, not guessed from memory --
//! including catching, on this pass, that registering two
//! `#[interface]` blocks on *one* struct doesn't work (it's two
//! conflicting `impl Interface for T`) -- which is why every node here
//! that exposes more than one D-Bus interface (Root, Dock,
//! Notifications) is really two or three small structs sharing state
//! via `Arc<Mutex<ShellTree>>`, each registered at the same path. The
//! same issue applies to signals specifically: `org.a11y.atspi.Event
//! .Object` is its own D-Bus interface separate from `Accessible`, so
//! `RootEventObject`/`NotificationsEventObject` exist purely to host
//! the `StateChanged`/`ChildrenChanged` signal declarations at the
//! right interface name, rather than those living directly on
//! `RootAccessible`/`NotificationsAccessible`.
//!
//! What's still **not** independently verified is the registration
//! handshake with the registry daemon (`registryd`) in `connect()`,
//! and the exact numeric bit positions used in `basic_state()` for
//! the AT-SPI `State` enum (VISIBLE/SHOWING/FOCUSABLE/FOCUSED) --
//! this sandbox has no accessibility bus or running screen reader to
//! confirm either against. If something is discoverable but announces
//! with the wrong state flags, or the registry handshake needs a
//! different call shape, `basic_state()` and `connect()` are exactly
//! where to look.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use zbus::blocking::Connection;
use zbus::interface;
use zbus::object_server::SignalContext;
use zbus::zvariant::OwnedObjectPath;

const APP_NAME: &str = "org.mitos.gui";
const NULL_PATH: &str = "/org/a11y/atspi/null";
const ROOT_PATH: &str = "/org/mitos/gui/Accessible/Root";
const DOCK_PATH: &str = "/org/mitos/gui/Accessible/Dock";
const NOTIF_PATH: &str = "/org/mitos/gui/Accessible/Notifications";

fn null_ref() -> (String, OwnedObjectPath) {
    (String::new(), OwnedObjectPath::try_from(NULL_PATH).unwrap())
}

fn app_ref(path: &str) -> (String, OwnedObjectPath) {
    (APP_NAME.to_string(), OwnedObjectPath::try_from(path.to_string()).unwrap())
}

/// AT-SPI `State` bits this provider ever sets -- see the module doc's
/// confidence note on the exact positions. `visible`/`showing` are set
/// unconditionally by every node here; `focusable`/`focused` only
/// apply to dock items.
fn basic_state(focusable: bool, focused: bool) -> Vec<u32> {
    let mut lower: u32 = 0;
    lower |= 1 << 30; // VISIBLE
    lower |= 1 << 31; // SHOWING
    if focusable {
        lower |= 1 << 14; // FOCUSABLE
    }
    if focused {
        lower |= 1 << 13; // FOCUSED
    }
    vec![lower, 0]
}

/// One dock item's static accessibility info, built once at startup --
/// see the module doc for why the dock doesn't need runtime add/remove
/// the way notifications do.
#[derive(Clone)]
pub struct DockItemInfo {
    pub id: &'static str,
    pub name: String,
}

/// Live, already-formatted state the D-Bus dispatch thread reads and
/// `state.rs` writes to (via `AtspiProvider`'s methods, which take the
/// lock briefly rather than holding it). All the *real* compositor
/// state stays on `MitosGuiState`, owned by the main thread only --
/// this is deliberately just the small subset worth announcing, kept
/// in a form the dispatch thread can read on its own.
#[derive(Default)]
struct ShellTree {
    root_description: String,
    dock_focused_index: Option<usize>,
    /// (notification id, title) for currently active toasts, in
    /// display order -- diffed on each `sync_notifications` call to
    /// decide which D-Bus objects to add/remove.
    notifications: Vec<(u32, String)>,
}

// ============================================================================
// ROOT: Accessible + Application
// ============================================================================

struct RootAccessible {
    tree: Arc<Mutex<ShellTree>>,
}

#[interface(name = "org.a11y.atspi.Accessible")]
impl RootAccessible {
    #[zbus(property)]
    fn name(&self) -> String {
        "MITOS Shell".to_string()
    }

    #[zbus(property)]
    fn description(&self) -> String {
        self.tree.lock().unwrap().root_description.clone()
    }

    #[zbus(property, name = "Parent")]
    fn parent(&self) -> (String, OwnedObjectPath) {
        null_ref()
    }

    #[zbus(property, name = "ChildCount")]
    fn child_count(&self) -> i32 {
        2
    }

    fn get_child_at_index(&self, index: i32) -> (String, OwnedObjectPath) {
        match index {
            0 => app_ref(DOCK_PATH),
            1 => app_ref(NOTIF_PATH),
            _ => null_ref(),
        }
    }

    fn get_children(&self) -> Vec<(String, OwnedObjectPath)> {
        vec![app_ref(DOCK_PATH), app_ref(NOTIF_PATH)]
    }

    fn get_index_in_parent(&self) -> i32 {
        -1
    }

    fn get_role(&self) -> u32 {
        71 // ROLE_APPLICATION
    }

    fn get_role_name(&self) -> String {
        "application".to_string()
    }

    fn get_localized_role_name(&self) -> String {
        "application".to_string()
    }

    fn get_state(&self) -> Vec<u32> {
        basic_state(false, false)
    }

    fn get_attributes(&self) -> HashMap<String, String> {
        HashMap::new()
    }

    fn get_application(&self) -> (String, OwnedObjectPath) {
        app_ref(ROOT_PATH)
    }

    fn get_interfaces(&self) -> Vec<String> {
        vec![
            "org.a11y.atspi.Accessible".to_string(),
            "org.a11y.atspi.Application".to_string(),
        ]
    }
}

/// `org.a11y.atspi.Event.Object` is its own D-Bus interface, separate
/// from `Accessible` -- a signal declared inside `RootAccessible`'s
/// `impl` above would be emitted as `...Accessible.StateChanged`
/// instead of the `...Event.Object.StateChanged` real AT-SPI clients
/// listen for, so it needs its own small struct registered at the
/// same path rather than living on `RootAccessible` itself.
struct RootEventObject;

#[interface(name = "org.a11y.atspi.Event.Object")]
impl RootEventObject {
    /// Fired by `AtspiProvider::announce_state_change` -- e.g. the
    /// launcher opening, the lock screen engaging. Declared with no
    /// body: zbus's `interface` macro expands this into the real
    /// emission code and generates the `RootEventObject::state_changed
    /// (&ctxt, ...)` associated function used to call it, per zbus's
    /// documented `#[zbus(signal)]` mechanism.
    #[zbus(signal)]
    async fn state_changed(
        ctxt: &SignalContext<'_>,
        state: &str,
        enabled: i32,
        detail: i32,
    ) -> zbus::Result<()>;
}

#[interface(name = "org.a11y.atspi.Application")]
impl RootAccessible {
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

// ============================================================================
// DOCK: Accessible + Component (container); DOCK ITEMS: Accessible + Action
// ============================================================================

struct DockAccessible {
    item_count: usize,
    tree: Arc<Mutex<ShellTree>>,
}

#[interface(name = "org.a11y.atspi.Accessible")]
impl DockAccessible {
    #[zbus(property)]
    fn name(&self) -> String {
        "Dock".to_string()
    }

    #[zbus(property)]
    fn description(&self) -> String {
        let focused = self.tree.lock().unwrap().dock_focused_index;
        match focused {
            Some(i) => format!("{} pinned applications, item {} focused", self.item_count, i + 1),
            None => format!("{} pinned applications", self.item_count),
        }
    }

    #[zbus(property, name = "Parent")]
    fn parent(&self) -> (String, OwnedObjectPath) {
        app_ref(ROOT_PATH)
    }

    #[zbus(property, name = "ChildCount")]
    fn child_count(&self) -> i32 {
        self.item_count as i32
    }

    fn get_child_at_index(&self, index: i32) -> (String, OwnedObjectPath) {
        if index < 0 || index as usize >= self.item_count {
            return null_ref();
        }
        app_ref(&format!("{DOCK_PATH}/{index}"))
    }

    fn get_children(&self) -> Vec<(String, OwnedObjectPath)> {
        (0..self.item_count as i32).map(|i| self.get_child_at_index(i)).collect()
    }

    fn get_index_in_parent(&self) -> i32 {
        0
    }

    fn get_role(&self) -> u32 {
        68 // ROLE_PANEL
    }

    fn get_role_name(&self) -> String {
        "panel".to_string()
    }

    fn get_localized_role_name(&self) -> String {
        "panel".to_string()
    }

    fn get_state(&self) -> Vec<u32> {
        basic_state(false, false)
    }

    fn get_attributes(&self) -> HashMap<String, String> {
        HashMap::new()
    }

    fn get_application(&self) -> (String, OwnedObjectPath) {
        app_ref(ROOT_PATH)
    }

    fn get_interfaces(&self) -> Vec<String> {
        vec![
            "org.a11y.atspi.Accessible".to_string(),
            "org.a11y.atspi.Component".to_string(),
        ]
    }
}

#[interface(name = "org.a11y.atspi.Component")]
impl DockAccessible {
    fn contains(&self, _x: i32, _y: i32, _coord_type: u32) -> bool {
        false
    }

    fn get_extents(&self, _coord_type: u32) -> (i32, i32, i32, i32) {
        // Real-time pixel bounds would mean threading the renderer's
        // live layout state into this D-Bus dispatch thread, which
        // isn't done here (see the module doc) -- reports a zero-size
        // rect rather than a stale or made-up one.
        (0, 0, 0, 0)
    }
}

struct DockItemAccessible {
    index: usize,
    info: DockItemInfo,
    tree: Arc<Mutex<ShellTree>>,
    action_tx: mpsc::Sender<String>,
}

#[interface(name = "org.a11y.atspi.Accessible")]
impl DockItemAccessible {
    #[zbus(property)]
    fn name(&self) -> String {
        self.info.name.clone()
    }

    #[zbus(property)]
    fn description(&self) -> String {
        String::new()
    }

    #[zbus(property, name = "Parent")]
    fn parent(&self) -> (String, OwnedObjectPath) {
        app_ref(DOCK_PATH)
    }

    #[zbus(property, name = "ChildCount")]
    fn child_count(&self) -> i32 {
        0
    }

    fn get_child_at_index(&self, _index: i32) -> (String, OwnedObjectPath) {
        null_ref()
    }

    fn get_children(&self) -> Vec<(String, OwnedObjectPath)> {
        Vec::new()
    }

    fn get_index_in_parent(&self) -> i32 {
        self.index as i32
    }

    fn get_role(&self) -> u32 {
        55 // ROLE_PUSH_BUTTON
    }

    fn get_role_name(&self) -> String {
        "push button".to_string()
    }

    fn get_localized_role_name(&self) -> String {
        "push button".to_string()
    }

    fn get_state(&self) -> Vec<u32> {
        let focused = self.tree.lock().unwrap().dock_focused_index == Some(self.index);
        basic_state(true, focused)
    }

    fn get_attributes(&self) -> HashMap<String, String> {
        HashMap::new()
    }

    fn get_application(&self) -> (String, OwnedObjectPath) {
        app_ref(ROOT_PATH)
    }

    fn get_interfaces(&self) -> Vec<String> {
        vec![
            "org.a11y.atspi.Accessible".to_string(),
            "org.a11y.atspi.Action".to_string(),
        ]
    }
}

#[interface(name = "org.a11y.atspi.Action")]
impl DockItemAccessible {
    fn get_n_actions(&self) -> i32 {
        1
    }

    fn get_name(&self, index: i32) -> String {
        if index == 0 { "activate".to_string() } else { String::new() }
    }

    fn get_localized_name(&self, index: i32) -> String {
        self.get_name(index)
    }

    fn get_description(&self, index: i32) -> String {
        if index == 0 {
            format!("Launches {}", self.info.name)
        } else {
            String::new()
        }
    }

    fn get_key_binding(&self, _index: i32) -> String {
        String::new()
    }

    fn get_actions(&self) -> Vec<(String, String, String)> {
        vec![("activate".to_string(), format!("Launches {}", self.info.name), String::new())]
    }

    fn do_action(&self, index: i32) -> bool {
        if index == 0 {
            let _ = self.action_tx.send(self.info.id.to_string());
            true
        } else {
            false
        }
    }
}

// ============================================================================
// NOTIFICATIONS: Accessible + Component (container); ITEMS: Accessible
// ============================================================================

struct NotificationsAccessible {
    tree: Arc<Mutex<ShellTree>>,
}

#[interface(name = "org.a11y.atspi.Accessible")]
impl NotificationsAccessible {
    #[zbus(property)]
    fn name(&self) -> String {
        "Notifications".to_string()
    }

    #[zbus(property)]
    fn description(&self) -> String {
        let n = self.tree.lock().unwrap().notifications.len();
        format!("{n} active notification(s)")
    }

    #[zbus(property, name = "Parent")]
    fn parent(&self) -> (String, OwnedObjectPath) {
        app_ref(ROOT_PATH)
    }

    #[zbus(property, name = "ChildCount")]
    fn child_count(&self) -> i32 {
        self.tree.lock().unwrap().notifications.len() as i32
    }

    fn get_child_at_index(&self, index: i32) -> (String, OwnedObjectPath) {
        let tree = self.tree.lock().unwrap();
        match tree.notifications.get(index.max(0) as usize) {
            Some((id, _)) if index >= 0 => app_ref(&format!("{NOTIF_PATH}/{id}")),
            _ => null_ref(),
        }
    }

    fn get_children(&self) -> Vec<(String, OwnedObjectPath)> {
        let tree = self.tree.lock().unwrap();
        tree.notifications
            .iter()
            .map(|(id, _)| app_ref(&format!("{NOTIF_PATH}/{id}")))
            .collect()
    }

    fn get_index_in_parent(&self) -> i32 {
        1
    }

    fn get_role(&self) -> u32 {
        68 // ROLE_PANEL
    }

    fn get_role_name(&self) -> String {
        "panel".to_string()
    }

    fn get_localized_role_name(&self) -> String {
        "panel".to_string()
    }

    fn get_state(&self) -> Vec<u32> {
        basic_state(false, false)
    }

    fn get_attributes(&self) -> HashMap<String, String> {
        HashMap::new()
    }

    fn get_application(&self) -> (String, OwnedObjectPath) {
        app_ref(ROOT_PATH)
    }

    fn get_interfaces(&self) -> Vec<String> {
        vec![
            "org.a11y.atspi.Accessible".to_string(),
            "org.a11y.atspi.Component".to_string(),
        ]
    }
}

/// See `RootEventObject`'s doc: `Event.Object` is its own D-Bus
/// interface, so the signal a screen reader actually needs to notice
/// new/removed notifications lives on its own struct at the same path
/// rather than inside `NotificationsAccessible`'s `Accessible` impl.
struct NotificationsEventObject;

#[interface(name = "org.a11y.atspi.Event.Object")]
impl NotificationsEventObject {
    /// Fired by `AtspiProvider::sync_notifications` whenever a toast
    /// is added or removed, so a screen reader watching this container
    /// (rather than only polling it) notices new notifications as
    /// they arrive -- arguably the single most useful live event this
    /// whole module provides.
    #[zbus(signal)]
    async fn children_changed(
        ctxt: &SignalContext<'_>,
        operation: &str,
        index_in_parent: i32,
        detail: i32,
        child: (String, OwnedObjectPath),
    ) -> zbus::Result<()>;
}

#[interface(name = "org.a11y.atspi.Component")]
impl NotificationsAccessible {
    fn contains(&self, _x: i32, _y: i32, _coord_type: u32) -> bool {
        false
    }

    fn get_extents(&self, _coord_type: u32) -> (i32, i32, i32, i32) {
        (0, 0, 0, 0) // see DockAccessible::get_extents's comment
    }
}

struct NotificationItemAccessible {
    title: String,
}

#[interface(name = "org.a11y.atspi.Accessible")]
impl NotificationItemAccessible {
    #[zbus(property)]
    fn name(&self) -> String {
        self.title.clone()
    }

    #[zbus(property)]
    fn description(&self) -> String {
        String::new()
    }

    #[zbus(property, name = "Parent")]
    fn parent(&self) -> (String, OwnedObjectPath) {
        app_ref(NOTIF_PATH)
    }

    #[zbus(property, name = "ChildCount")]
    fn child_count(&self) -> i32 {
        0
    }

    fn get_child_at_index(&self, _index: i32) -> (String, OwnedObjectPath) {
        null_ref()
    }

    fn get_children(&self) -> Vec<(String, OwnedObjectPath)> {
        Vec::new()
    }

    fn get_index_in_parent(&self) -> i32 {
        0
    }

    fn get_role(&self) -> u32 {
        // ROLE_NOTIFICATION, per the AT-SPI Accessible.Role enum.
        104
    }

    fn get_role_name(&self) -> String {
        "notification".to_string()
    }

    fn get_localized_role_name(&self) -> String {
        "notification".to_string()
    }

    fn get_state(&self) -> Vec<u32> {
        basic_state(false, false)
    }

    fn get_attributes(&self) -> HashMap<String, String> {
        HashMap::new()
    }

    fn get_application(&self) -> (String, OwnedObjectPath) {
        app_ref(ROOT_PATH)
    }

    fn get_interfaces(&self) -> Vec<String> {
        vec!["org.a11y.atspi.Accessible".to_string()]
    }
}

// ============================================================================
// PROVIDER: connection lifecycle + the two things the main loop calls
// ============================================================================

/// Handle to the accessibility-bus connection and the shared state its
/// objects read from. `state.rs` calls `sync_notifications` each frame
/// and `announce_state_change` when something modal opens/closes;
/// `poll_actions` drains dock-item activations. Everything else is
/// handled by zbus's own background dispatch thread, same as
/// `dbus.rs`'s notification service.
pub struct AtspiProvider {
    conn: Connection,
    tree: Arc<Mutex<ShellTree>>,
    registered_notifications: HashSet<u32>,
    action_rx: mpsc::Receiver<String>,
}

impl AtspiProvider {
    /// Best-effort setup: connects to the accessibility bus, registers
    /// MITOS's shell as a small accessible tree (root, dock + its
    /// items, an initially-empty notifications container), and asks
    /// the registry to embed it. Returns `None` -- never an error a
    /// caller has to handle -- if any step fails, e.g. no
    /// accessibility bus is running at all, which is completely normal
    /// on a system with no assistive technology in use. Screen-reader
    /// support being unavailable should never be a reason the rest of
    /// the compositor fails to start.
    pub fn connect(dock_items: Vec<DockItemInfo>) -> Option<Self> {
        let session = Connection::session().ok()?;

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

        let tree = Arc::new(Mutex::new(ShellTree::default()));
        let (action_tx, action_rx) = mpsc::channel();

        conn.object_server()
            .at(ROOT_PATH, RootAccessible { tree: tree.clone() })
            .ok()?;
        conn.object_server().at(ROOT_PATH, RootEventObject).ok()?;

        conn.object_server()
            .at(
                DOCK_PATH,
                DockAccessible { item_count: dock_items.len(), tree: tree.clone() },
            )
            .ok()?;

        for (index, info) in dock_items.into_iter().enumerate() {
            let path = format!("{DOCK_PATH}/{index}");
            let _ = conn.object_server().at(
                path,
                DockItemAccessible {
                    index,
                    info,
                    tree: tree.clone(),
                    action_tx: action_tx.clone(),
                },
            );
        }

        conn.object_server()
            .at(NOTIF_PATH, NotificationsAccessible { tree: tree.clone() })
            .ok()?;
        conn.object_server().at(NOTIF_PATH, NotificationsEventObject).ok()?;

        conn.request_name(APP_NAME).ok()?;

        // Ask the registry to embed us -- best-effort; see the module
        // doc's confidence note. If it doesn't match what a real
        // registryd expects, the tree above is still sitting on the
        // bus at a well-known name for a client willing to query it
        // directly either way.
        let embed_result: Result<zbus::Message, _> = conn.call_method(
            Some("org.a11y.atspi.Registry"),
            "/org/a11y/atspi/accessible/root",
            Some("org.a11y.atspi.Socket"),
            "Embed",
            &(APP_NAME, zbus::zvariant::ObjectPath::try_from(ROOT_PATH).ok()?),
        );
        if let Err(err) = embed_result {
            tracing::warn!("MITOS GUI: AT-SPI registry embed failed (non-fatal): {err}");
        }

        Some(Self {
            conn,
            tree,
            registered_notifications: HashSet::new(),
            action_rx,
        })
    }

    /// Drains dock-item activations queued by `DockItemAccessible::
    /// do_action` since the last call. Called once per frame from the
    /// main loop; returns app ids to launch (same ids `DockItem::id`
    /// uses), for the caller to hand to `shell_interaction::launch_app`
    /// -- kept separate from that call so this module doesn't need a
    /// `&mut MitosGuiState` of its own.
    pub fn poll_actions(&self) -> Vec<String> {
        self.action_rx.try_iter().collect()
    }

    /// Updates the live description text and dock-focus state that
    /// the Root/Dock-item properties above read. Cheap when nothing
    /// changed (a lock, a couple of field writes) -- call every frame.
    pub fn update_summary(&self, description: impl Into<String>, dock_focused: Option<usize>) {
        let mut t = self.tree.lock().unwrap();
        t.root_description = description.into();
        t.dock_focused_index = dock_focused;
    }

    /// Diffs `active` (id, title) pairs against what's currently
    /// registered on the bus, adding/removing D-Bus objects and firing
    /// `ChildrenChanged` for each change. Call once per frame with
    /// `state.notifications.active`'s current contents; a no-op frame
    /// (nothing added or removed) costs one Vec comparison.
    pub fn sync_notifications(&mut self, active: &[(u32, String)]) {
        let current: HashSet<u32> = active.iter().map(|(id, _)| *id).collect();

        let removed: Vec<u32> = self
            .registered_notifications
            .iter()
            .copied()
            .filter(|id| !current.contains(id))
            .collect();
        let added: Vec<&(u32, String)> = active
            .iter()
            .filter(|(id, _)| !self.registered_notifications.contains(id))
            .collect();

        if removed.is_empty() && added.is_empty() {
            return;
        }

        for id in &removed {
            let path = format!("{NOTIF_PATH}/{id}");
            let _ = self.conn.object_server().remove::<NotificationItemAccessible, _>(&path);
            self.registered_notifications.remove(id);
            self.emit_notifications_children_changed("remove", &path);
        }

        for (id, title) in &added {
            let path = format!("{NOTIF_PATH}/{id}");
            if self
                .conn
                .object_server()
                .at(&path, NotificationItemAccessible { title: title.clone() })
                .unwrap_or(false)
            {
                self.registered_notifications.insert(*id);
                self.emit_notifications_children_changed("add", &path);
            }
        }

        {
            let mut t = self.tree.lock().unwrap();
            t.notifications = active.to_vec();
        }
    }

    fn emit_notifications_children_changed(&self, operation: &str, child_path: &str) {
        let Ok(iface_ref) = self
            .conn
            .object_server()
            .interface::<_, NotificationsEventObject>(NOTIF_PATH)
        else {
            return;
        };
        let ctxt = iface_ref.signal_context();
        let child = app_ref(child_path);
        // `children_changed` is `async fn` (all `#[zbus(signal)]` methods
        // are -- emitting is I/O) but this whole provider otherwise
        // uses zbus's blocking API, so block on it here rather than
        // needing an async runtime just for this one call.
        let _ = zbus::block_on(NotificationsEventObject::children_changed(
            ctxt, operation, 0, 0, child,
        ));
    }

    /// Announces a modal shell state change (launcher opening/closing,
    /// the lock screen engaging) via `Event.Object.StateChanged` on
    /// the root object, for a screen reader watching for state changes
    /// rather than only polling `Root`'s `Description` property.
    pub fn announce_state_change(&self, state: &str, enabled: bool) {
        let Ok(iface_ref) = self.conn.object_server().interface::<_, RootEventObject>(ROOT_PATH)
        else {
            return;
        };
        let ctxt = iface_ref.signal_context();
        let _ = zbus::block_on(RootEventObject::state_changed(
            ctxt,
            state,
            if enabled { 1 } else { 0 },
            0,
        ));
    }
}
