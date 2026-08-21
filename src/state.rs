//! Global state for the MITOS compositor.

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
    wayland::{
        compositor::CompositorState,
        shell::xdg::XdgShellState,
        shm::ShmState,
    },
};

pub struct MitosGuiState {
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<Self>,
    pub seat: Seat<Self>,
    pub output: Output,

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
}

impl MitosGuiState {
    pub fn new(
        compositor_state: CompositorState,
        xdg_shell_state: XdgShellState,
        shm_state: ShmState,
        seat_state: SeatState<Self>,
        seat: Seat<Self>,
        output: Output,
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
            space,
            popups: PopupManager::default(),
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
