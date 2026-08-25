//! Global state for the MITOS compositor.

use crate::desktop::HomeScreenConfig;
use crate::renderer::GlassPanel;

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
}

impl MitosShell {
    pub fn new() -> Self {
        Self {
            top_bar: None,
            launcher: None,
            dock: None,
            dock_layout: crate::desktop::DockLayout::default(),
            launcher_visible: false,
        }
    }

    /// Recalculate shell geometry for the current output.
    pub fn update_layout(
        &mut self,
        config: &HomeScreenConfig,
        output_size: smithay::utils::Size<
            i32,
            smithay::utils::Logical,
        >,
    ) {
        let layout =
            crate::desktop::ShellLayout::calculate(
                config,
                output_size,
            );

        self.top_bar = layout.top_bar;
        self.launcher = layout.launcher;
        self.dock = layout.dock;
    }

    /// Toggle the launcher visibility.
   /// Toggle the launcher visibility.
pub fn toggle_launcher(&mut self) {
    self.launcher_visible = !self.launcher_visible;
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
        //
        // The output starts at logical coordinate (0, 0).
        // Client windows will later be mapped into this space.
        // ------------------------------------------------------------

        let mut space = Space::default();

        space.map_output(&output, (0, 0));


        let mut shell = MitosShell::new();

shell.update_layout(
    &home_screen,
    output.current_logical_size(),
);
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
      }
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
        // Window focus handling will be implemented during
        // the Stage 4 window-manager work.
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
