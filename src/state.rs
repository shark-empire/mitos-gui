//! Global state for the MITOS compositor.

use smithay::wayland::{
    compositor::CompositorState,
    shm::ShmState,
    shell::xdg::XdgShellState,
};

pub struct MitosGuiState {
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
}

impl MitosGuiState {
    pub fn new(
        compositor_state: CompositorState,
        xdg_shell_state: XdgShellState,
        shm_state: ShmState,
    ) -> Self {
        Self {
            compositor_state,
            xdg_shell_state,
            shm_state,
        }
    }
}
