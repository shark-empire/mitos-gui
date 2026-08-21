//! Global state for the MITOS compositor.

use smithay::wayland::compositor::CompositorState;
use smithay::wayland::shell::xdg::XdgShellState;

pub struct MitosGuiState {
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
}

impl MitosGuiState {
    pub fn new(
        compositor_state: CompositorState,
        xdg_shell_state: XdgShellState,
    ) -> Self {
        Self {
            compositor_state,
            xdg_shell_state,
        }
    }
}
