//! Global state for the MITOS compositor.
use crate::renderer::GlassPanel;
use crate::renderer::MitosShell;

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

use crate::desktop::HomeScreenConfig;

pub struct MitosGuiState {
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<Self>,
    pub seat: Seat<Self>,
    pub shell: MitosShell,
    pub output: Output,
    
    


    
pub struct MitosShell {
    pub top_bar: Option<GlassPanel>,

    pub launcher: Option<GlassPanel>,

    pub dock: Option<GlassPanel>,
}

impl MitosShell {
    pub fn new() -> Self {
        Self {
            top_bar: None,
            launcher: None,
            dock: None,
        }
    }
}
    // --------------------------------------------------------
    // Home screen (Stage 3)
    //
    // Wallpaper/background color, loaded once at startup by
    // `desktop::HomeScreenConfig::load()` and read every frame by
    // `renderer::clear_color()`. Panels and the top bar grow this
    // struct rather than becoming state fields of their own.
    // --------------------------------------------------------
    pub home_screen: HomeScreenConfig,

    // --------------------------------------------------------
    // Desktop tracking
    //
    // `space` is the 2D plane windows and outputs are mapped onto.
    // It's the single source of truth for "what exists and where" —
    // the renderer (Stage 2) will iterate it to know what to draw,
    // and the window manager (Stage 4) will move things around in it.
    //
    // `popups` tracks xdg_popup surfaces (menus, tooltips) so their
    // lifecycle and positioning can be resolved against their parent.
    // --------------------------------------------------------
    pub space: Space<Window>,
    pub popups: PopupManager,

    // --------------------------------------------------------
    // Input (Stage 2)
    //
    // `pointer_location` is the single source of truth for "where is
    // the cursor right now" -- input.rs updates it on every motion
    // event, and the renderer (once Stage 3 draws a cursor sprite
    // instead of relying on the host's) will read it from here too.
    //
    // `clock` backs the timestamps handed to clients in frame-done
    // callbacks after each render, so their `wl_surface.frame`
    // requests resolve against a real monotonic clock rather than
    // whatever `Instant` happened to be lying around.
    // --------------------------------------------------------
    pub pointer_location: Point<f64, Logical>,
    pub clock: Clock<Monotonic>,
    pub top_bar_panel: Option<GlassPanel>,
}

impl MitosGuiState {
    pub fn new(
        compositor_state: CompositorState,
        xdg_shell_state: XdgShellState,
        shm_state: ShmState,
        seat_state: SeatState<Self>,
        seat: Seat<Self>,
        output: Output,
        home_screen: HomeScreenConfig,
        top_bar_panel: Option<GlassPanel>,
    ) -> Self {
        let mut space = Space::default();
        space.map_output(&output, (0, 0));

        Self {
            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
            seat,
            output,
            home_screen,
            space,
            top_bar_panel,
            popups: PopupManager::default(),
            pointer_location: (0.0, 0.0).into(),
            clock: Clock::new(),
            shell: MitosShell::new(),
        }
    }
}

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
    }

    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        _image: CursorImageStatus,
    ) {
    }
}
