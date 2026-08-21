//! MITOS Wayland compositor integration.

use smithay::wayland::compositor::{
    CompositorClientState,
    CompositorHandler,
    CompositorState,
};
use smithay::wayland::shell::xdg::XdgShellState;

use smithay::reexports::wayland_server::{
    backend::Client,
    protocol::wl_surface::WlSurface,
};

use crate::state::MitosGuiState;

impl CompositorHandler for MitosGuiState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(
        &self,
        client: &'a Client,
    ) -> &'a CompositorClientState {
        client
            .get_data::<MitosClientState>()
            .expect("MITOS client state missing")
            .compositor_state()
    }

    fn commit(&mut self, _surface: &WlSurface) {
        // Surface commits will eventually trigger:
        //
        // 1. Surface state updates
        // 2. Window damage tracking
        // 3. Rendering
        // 4. Frame callbacks
        //
        // For now this is intentionally minimal.
    }
}

pub struct MitosClientState {
    compositor_state: CompositorClientState,
}

impl MitosClientState {
    pub fn new() -> Self {
        Self {
            compositor_state: CompositorClientState::default(),
        }
    }

    pub fn compositor_state(&self) -> &CompositorClientState {
        &self.compositor_state
    }
}
