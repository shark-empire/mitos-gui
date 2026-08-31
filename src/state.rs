//! Global state for the MITOS compositor.

use crate::desktop::HomeScreenConfig;
use crate::renderer::GlassPanel;
use crate::shell_interaction::AppEntry;
use crate::wm::InteractiveAction;

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

    /// Set by the DRM vblank handler; consumed by the DRM main loop.
    pub drm_vblank: bool,

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
///
/// This is the central state object shared by:
///
/// - the Wayland compositor
/// - XDG shell
/// - input handling
/// - window management
/// - renderer
/// - MITOS shell
/// - desktop/background
pub struct MitosGuiState {
    // ------------------------------------------------------------------------
    // Wayland protocol state
    // ------------------------------------------------------------------------

    /// Wayland compositor state.
    pub compositor_state: CompositorState,

    /// XDG shell state.
    pub xdg_shell_state: XdgShellState,

    /// Shared-memory buffer state.
    pub shm_state: ShmState,

    // ------------------------------------------------------------------------
    // Input
    // ------------------------------------------------------------------------

    /// Wayland seat state.
    pub seat_state: SeatState<Self>,

    /// MITOS seat containing keyboard and pointer capabilities.
    pub seat: Seat<Self>,

    // ------------------------------------------------------------------------
    // MITOS shell
    // ------------------------------------------------------------------------

    /// Visual shell state.
    pub shell: MitosShell,

    // ------------------------------------------------------------------------
    // Output
    // ------------------------------------------------------------------------

    /// Current compositor output.
    pub output: Output,

    // ------------------------------------------------------------------------
    // Desktop
    // ------------------------------------------------------------------------

    /// Home-screen and wallpaper configuration.
    pub home_screen: HomeScreenConfig,

    /// Desktop space containing outputs and client windows.
    pub space: Space<Window>,

    /// XDG popup manager.
    pub popups: PopupManager,

    // ------------------------------------------------------------------------
    // Pointer / timing
    // ------------------------------------------------------------------------

    /// Current pointer position in logical coordinates.
    pub pointer_location: Point<f64, Logical>,

    /// Monotonic compositor clock.
    pub clock: Clock<Monotonic>,

    /// Currently focused window.
    pub focused_window: Option<Window>,

    /// Minimized windows (most recent last).
    pub minimized: Vec<Window>,

    /// Active interactive move/resize gesture.
    pub interactive: Option<InteractiveAction>,


    /// Stage 6: Notification engine.
    pub notifications: crate::notifications::NotificationManager,

    /// Discovered applications for the launcher.
    pub launcher_apps: Vec<AppEntry>,

    /// Set by the config watcher; consumed by the main loop to force
    /// a full redraw and shader recompile after a live config reload.
    pub pending_full_redraw: bool,
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

         notifications: crate::notifications::NotificationManager::new(),

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
            launcher_apps,
            pending_full_redraw: false,
            
        }
    }

    /// Reload ~/.config/mitos/home.conf and recompute the shell.
    ///
    /// Called from the config-watcher channel when the file changes.
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

    /// Return the compositor's seat state.
    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    /// Called whenever keyboard/pointer focus changes.
    fn focus_changed(
        &mut self,
        _seat: &Seat<Self>,
        _focused: Option<&WlSurface>,
    ) {
        // Handled by wm::set_focus
    }

    /// Called when a client changes its cursor image.
    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        _image: CursorImageStatus,
    ) {
        // MITOS cursor rendering will be implemented later.
    }
}
