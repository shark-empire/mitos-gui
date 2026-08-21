//! MITOS Wayland compositor integration.

use smithay::{
    delegate_compositor,
    delegate_shm,
    delegate_xdg_shell,
};

use smithay::wayland::{
    buffer::BufferHandler,
    compositor::{
        CompositorClientState,
        CompositorHandler,
        CompositorState,
    },
    shm::{ShmHandler, ShmState},
    shell::xdg::{
        PopupSurface,
        PositionerState,
        ToplevelSurface,
        XdgShellHandler,
        XdgShellState,
    },
};

use smithay::reexports::wayland_server::{
    backend::{ClientData, ClientId, DisconnectReason},
    protocol::{
        wl_buffer,
        wl_surface::WlSurface,
    },
    Client,
};

use smithay::utils::Serial;

use crate::state::MitosGuiState;

use wayland_protocols::xdg::shell::server::xdg_toplevel;

use smithay::reexports::wayland_server::protocol::wl_seat;


// ============================================================
// XDG SHELL
// ============================================================

impl XdgShellHandler for MitosGuiState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        println!("MITOS GUI: new application window");

        surface.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Activated);
        });

        surface.send_configure();
    }

    fn new_popup(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
    ) {
        println!("MITOS GUI: new popup");
    }

    fn grab(
        &mut self,
        _surface: PopupSurface,
        _seat: wl_seat::WlSeat,
        _serial: Serial,
    ) {
        println!("MITOS GUI: popup grab");
    }

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
        println!("MITOS GUI: popup reposition request");
    }
}


// ============================================================
// SHM
// ============================================================

impl ShmHandler for MitosGuiState {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}


// ============================================================
// BUFFER
// ============================================================

impl BufferHandler for MitosGuiState {
    fn buffer_destroyed(
        &mut self,
        _buffer: &wl_buffer::WlBuffer,
    ) {
    }
}


// ============================================================
// COMPOSITOR
// ============================================================

impl CompositorHandler for MitosGuiState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(
        &self,
        client: &'a Client,
    ) -> &'a CompositorClientState {
        &client
            .get_data::<MitosClientState>()
            .expect("MITOS GUI: missing client state")
            .compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
    }
}


// ============================================================
// CLIENT STATE
// ============================================================

#[derive(Default)]
pub struct MitosClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for MitosClientState {
    fn initialized(&self, _client_id: ClientId) {
        println!("MITOS GUI: Wayland client connected");
    }

    fn disconnected(
        &self,
        _client_id: ClientId,
        _reason: DisconnectReason,
    ) {
        println!("MITOS GUI: Wayland client disconnected");
    }
}


// ============================================================
// SMITHAY DELEGATES
// ============================================================

delegate_xdg_shell!(MitosGuiState);
delegate_compositor!(MitosGuiState);
delegate_shm!(MitosGuiState);
