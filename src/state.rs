use smithay::{
    input::SeatState,
    wayland::{
        compositor::CompositorState,
        shm::ShmState,
        shell::xdg::XdgShellState,
    },
};

pub struct MitosGuiState {
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<Self>,
}

impl MitosGuiState {
    pub fn new(
        compositor_state: CompositorState,
        xdg_shell_state: XdgShellState,
        shm_state: ShmState,
        seat_state: SeatState<Self>,
    ) -> Self {
        Self {
            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
        }
    }
}
