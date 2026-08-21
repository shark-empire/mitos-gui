//! Global state for the MITOS compositor.

use smithay::{
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
        Self {
            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
            seat,
            output,
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
