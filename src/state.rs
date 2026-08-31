//! Global state for the MITOS compositor.

use crate::desktop::HomeScreenConfig;
use crate::renderer::GlassPanel;
use crate::shell_interaction::AppEntry;
use crate::wm::InteractiveAction;

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

    // ------------------------------------------------------------------------
    // MITOS shell
    // ------------------------------------------------------------------------
    pub shell: MitosShell,

    // ------------------------------------------------------------------------
    // Output
    // ------------------------------------------------------------------------
    pub output: Output,

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

}

impl MitosGuiState {
    /// Create the initial MITOS compositor state.
    pub fn new(
        compositor_state: CompositorState,
        xdg_shell_state: XdgShellState,
        shm_state: ShmState,
        seat_state: SeatState<Self>,
        seat: Seat<Self>,
        output: Output,
        home_screen: HomeScreenConfig,
        _unused: Option<()>,
    ) -> Self {
        // ------------------------------------------------------------
        // Desktop space
        // ------------------------------------------------------------
        let mut space = Space::default();
        space.map_output(&output, (0, 0));

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
        }
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

        self.home_screen = HomeScreenConfig::load();
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
