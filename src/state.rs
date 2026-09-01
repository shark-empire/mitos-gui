//! Global state for the MITOS compositor.

use crate::desktop::HomeScreenConfig;
use crate::renderer::GlassPanel;
use crate::shell_interaction::AppEntry;
use crate::wm::InteractiveAction;

use std::collections::HashMap;
use std::time::{Duration, Instant};
use smithay::{
    desktop::{PopupManager, Space, Window},
    input::{
        pointer::CursorImageStatus,
        Seat,
        SeatHandler,
        SeatState,
    },
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Clock, Logical, Monotonic, Point},
    wayland::{
        compositor::CompositorState,
        shell::xdg::XdgShellState,
        shm::ShmState,
    },
};

// ============================================================================
// ON-SCREEN DISPLAY (OSD)
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OsdIcon {
    Volume,
    Muted,
    Brightness,
}

#[derive(Clone, Debug)]
pub struct OsdState {
    pub active: bool,
    pub icon: OsdIcon,
    pub value: f32, // 0.0 to 1.0
    pub last_updated: Instant,
}

impl OsdState {
    pub fn new() -> Self {
        Self {
            active: false,
            icon: OsdIcon::Volume,
            value: 0.0,
            // Hide immediately on startup by setting it to the past
            last_updated: Instant::now() - Duration::from_secs(3),
        }
    }

    pub fn trigger(&mut self, icon: OsdIcon, value: f32) {
        self.active = true;
        self.icon = icon;
        self.value = value.clamp(0.0, 1.0);
        self.last_updated = Instant::now();
    }
}

// ============================================================================
// MITOS SHELL
// ============================================================================

/// State owned by the MITOS visual shell.
///
/// The renderer is responsible for drawing these components.
/// This struct is responsible only for describing their state and geometry.
#[derive(Debug)]
pub struct MitosShell {
    /// Main MITOS top bar.
    pub top_bar: Option<GlassPanel>,

    /// Application launcher panel.
    pub launcher: Option<GlassPanel>,

    /// Desktop dock.
    pub dock: Option<GlassPanel>,

    pub dock_layout: crate::desktop::DockLayout,

    /// Whether the launcher is currently visible.
    pub launcher_visible: bool,

    // --- Launcher Search State ---
    pub launcher_query: String,
    pub launcher_results: Vec<AppEntry>,
    pub launcher_selected: usize,
}

impl MitosShell {
    pub fn new() -> Self {
        Self {
            top_bar: None,
            launcher: None,
            dock: None,
            dock_layout: crate::desktop::DockLayout::default(),
            launcher_visible: false,
            launcher_query: String::new(),
            launcher_results: Vec::new(),
            launcher_selected: 0,
        }
    }

    /// Recalculate shell geometry for the current output.
    pub fn update_layout(
        &mut self,
        config: &HomeScreenConfig,
        output_size: smithay::utils::Size<i32, smithay::utils::Logical>,
    ) {
        let layout = crate::desktop::ShellLayout::calculate(config, output_size);

        self.top_bar = layout.top_bar;
        self.launcher = layout.launcher;
        self.dock = layout.dock;
    }

    /// Toggle the launcher visibility.
    pub fn toggle_launcher(&mut self) {
        self.launcher_visible = !self.launcher_visible;
        if self.launcher_visible {
            // Reset state when opening
            self.launcher_query.clear();
            self.launcher_results = crate::shell_interaction::discover_apps();
            self.launcher_selected = 0;
        }
    }
}

// ============================================================================
// GLOBAL COMPOSITOR STATE
// ============================================================================

/// Global state for the MITOS compositor.
pub struct MitosGuiState {
    // ------------------------------------------------------------------------
    // Wayland protocol state
    // ------------------------------------------------------------------------
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,

    // ------------------------------------------------------------------------
    // Input
    // ------------------------------------------------------------------------
    pub seat_state: SeatState<Self>,
    pub seat: Seat<Self>,

    pub current_workspace: HashMap<String, usize>,
    
    // ------------------------------------------------------------------------
    // MITOS shell
    // ------------------------------------------------------------------------
    pub shell: MitosShell,

    // ------------------------------------------------------------------------
    // Output
    // ------------------------------------------------------------------------

     pub outputs: Vec<Output>,


    // ------------------------------------------------------------------------
    // Desktop
    // ------------------------------------------------------------------------
    pub home_screen: HomeScreenConfig,
    pub space: Space<Window>,
    pub popups: PopupManager,

    // ------------------------------------------------------------------------
    // Pointer / timing
    // ------------------------------------------------------------------------
    pub pointer_location: Point<f64, Logical>,
    pub clock: Clock<Monotonic>,
    pub focused_window: Option<Window>,
    pub minimized: Vec<Window>,
    pub interactive: Option<InteractiveAction>,

    // ------------------------------------------------------------------------
    // Stage 6: System Services
    // ------------------------------------------------------------------------
    pub notifications: crate::notifications::NotificationManager,
    pub network: crate::status::NetworkStatus,
    pub battery: Option<crate::status::BatteryStatus>,
    pub volume: u8,
    pub muted: bool,
    pub last_status_poll: Instant,

    /// Set by the DRM vblank handler; consumed by the DRM main loop.
    pub drm_vblank: bool,

    /// Discovered applications for the launcher.
    pub launcher_apps: Vec<AppEntry>,

    /// Set by the config watcher; consumed by the main loop to force
    /// a full redraw and shader recompile after a live config reload.
    pub pending_full_redraw: bool,

    /// Stage 6: Secure authentication prompt.
    pub auth: crate::auth::AuthPrompt,

    // ------------------------------------------------------------------------
    // Stage 7: OSD, Night Light, Hot Corners
    // ------------------------------------------------------------------------
    pub osd: OsdState,
    pub night_light: bool,
    pub hot_corners_last_triggered: Instant,

    pub pending_screenshot: bool,
    pub dbus_service: Option<crate::dbus::DbusService>,

}

impl MitosGuiState {
    /// Create the initial MITOS compositor state.
    pub fn new(
        compositor_state: CompositorState,
        xdg_shell_state: XdgShellState,
        shm_state: ShmState,
        seat_state: SeatState<Self>,
        seat: Seat<Self>,
        outputs: Vec<Output>,
        home_screen: HomeScreenConfig,
        dbus_service: crate::dbus::DbusService, // Added
        _unused: Option<()>,
    ) -> Self {
        // ------------------------------------------------------------
        // Desktop space
        // ------------------------------------------------------------
        let mut space = Space::default();
        let mut offset_x = 0;
        for o in &outputs {
            space.map_output(o, (offset_x, 0));
            if let Some(mode) = o.current_mode() {
                offset_x += mode.size.w;
            }
        }

     // Initialize the HashMap
        let mut current_workspace = HashMap::new();
        for o in &outputs {
            current_workspace.insert(o.name(), 0);
        }

        let mut shell = MitosShell::new();
        let output_size = output
            .current_mode()
            .map(|mode| {
                smithay::utils::Size::<i32, smithay::utils::Logical>::from((
                    mode.size.w,
                    mode.size.h,
                ))
            })
            .unwrap_or_else(|| {
                smithay::utils::Size::<i32, smithay::utils::Logical>::new(1280, 720)
            });

        shell.update_layout(&home_screen, output_size);

        // ------------------------------------------------------------
        // Pre-discover apps for the launcher search
        // ------------------------------------------------------------
        let launcher_apps = crate::shell_interaction::discover_apps();
        
        // Pre-populate launcher results so it's ready immediately when opened
        shell.launcher_results = launcher_apps.clone();

        let initial_night_light = home_screen.night_light;

        // ------------------------------------------------------------
        // Global state
        // ------------------------------------------------------------
        Self {
            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
            seat,
            shell,
            output,
            home_screen,
            space,
            popups: PopupManager::default(),
            pointer_location: (0.0, 0.0).into(),
            clock: Clock::new(),
            focused_window: None,
            minimized: Vec::new(),
            interactive: None,
            
            // Stage 6 Services
            notifications: crate::notifications::NotificationManager::new(),
            network: crate::status::NetworkStatus::Offline,
            battery: None,
            volume: 70,
            muted: false,
            last_status_poll: Instant::now(),
            drm_vblank: false,
            auth: crate::auth::AuthPrompt::new(),
            launcher_apps,
            pending_full_redraw: false,

            // Stage 7 Features
            osd: OsdState::new(),
            night_light: initial_night_light,
            hot_corners_last_triggered: Instant::now() - Duration::from_secs(1),

            pending_screenshot: false,
            dbus_service: Some(dbus_service),
        }
    }

    /// Toggle the Night Light (Eye Comfort) mode.
    pub fn toggle_night_light(&mut self) {
        self.night_light = !self.night_light;
        self.pending_full_redraw = true;
        println!("MITOS GUI: Night Light {}", if self.night_light { "enabled" } else { "disabled" });
    }

    /// Re-poll kernel status every 5 seconds.
    /// Returns true if anything changed (triggers a redraw).
    pub fn poll_status_if_due(&mut self) -> bool {
        if self.last_status_poll.elapsed() < Duration::from_secs(5) {
            return false;
        }

        self.last_status_poll = Instant::now();

        let net = crate::status::poll_network();
        let bat = crate::status::poll_battery();

        let changed = net != self.network || bat != self.battery;

        self.network = net;
        self.battery = bat;

        changed
    }

    /// Reload ~/.config/mitos/home.conf and recompute the shell.
    pub fn reload_configuration(&mut self) {
        println!("MITOS GUI: home.conf changed, reloading configuration");

        let old_config = self.home_screen.clone();
        self.home_screen = HomeScreenConfig::load();
        
        // Sync night light state if it was changed externally by mitos-settings
        if self.home_screen.night_light != old_config.night_light {
            self.night_light = self.home_screen.night_light;
        }

        crate::theme::MitosTheme::apply_runtime(&self.home_screen);

        let output_size = self
            .output
            .current_mode()
            .map(|mode| {
                smithay::utils::Size::<i32, smithay::utils::Logical>::from((
                    mode.size.w,
                    mode.size.h,
                ))
            })
            .unwrap_or_else(|| {
                smithay::utils::Size::<i32, smithay::utils::Logical>::new(1280, 720)
            });

        self.shell.update_layout(&self.home_screen, output_size);
        self.pending_full_redraw = true;
    }

// In impl MitosGuiState

pub fn add_output(&mut self, output: Output) {
    let mut offset_x = 0;
    for o in &self.outputs {
        if let Some(mode) = o.current_mode() {
            offset_x += mode.size.w;
        }
    }
    self.space.map_output(&output, (offset_x, 0));
            
    // Start new monitors on workspace 0
     self.current_workspace.entry(output.name()).or_insert(0);
    self.outputs.push(output);
}

pub fn remove_output(&mut self, output: &Output) {
    self.space.unmap_output(output);
    self.outputs.retain(|o| o != output);

     // Clean up workspace tracking for the removed monitor
    self.current_workspace.remove(&output.name());
    
    // If we removed the primary output, try to focus a window on the remaining one
    if self.outputs.is_empty() {
        self.focused_window = None;
    } else if let Some(focused) = &self.focused_window {
        // Check if focused window is still on a valid output
        let still_valid = self.space.outputs_for_element(focused).any(|o| o == output);
        if !still_valid {
            self.focused_window = None;
        }
    }
}


    pub fn poll_dbus(&mut self) {
        if let Some(service) = self.dbus_service.as_ref() {
            while let Ok((app_name, title, body)) = service.rx.try_recv() {
                self.notifications.push(&app_name, &title, &body);
                self.pending_full_redraw = true;
            }
        }
    }

        /// Switches the workspace on a specific monitor
    pub fn switch_workspace(&mut self, output_name: &str, ws: usize) {
        self.current_workspace.insert(output_name.to_string(), ws);
        self.pending_full_redraw = true;
    }

    /// Finds which monitor the pointer is currently hovering over
    pub fn active_output_name(&self) -> String {
        for output in &self.outputs {
            if let Some(geom) = self.space.output_geometry(output) {
                let ptr = self.pointer_location;
                if ptr.x >= geom.loc.x as f64 && ptr.x < (geom.loc.x + geom.size.w) as f64 &&
                   ptr.y >= geom.loc.y as f64 && ptr.y < (geom.loc.y + geom.size.h) as f64 {
                    return output.name();
                }
            }
        }
        self.outputs.first().map(|o| o.name()).unwrap_or_default()
    }


}

// ============================================================================
// SEAT HANDLER
// ============================================================================

impl SeatHandler for MitosGuiState {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(
        &mut self,
        _seat: &Seat<Self>,
        _focused: Option<&WlSurface>,
    ) {
        // Handled by wm::set_focus
    }

    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        _image: CursorImageStatus,
    ) {
        // MITOS cursor rendering will be implemented later.
    }
}
