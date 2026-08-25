//! MITOS Wayland compositor integration.

use smithay::{
    delegate_compositor,
    delegate_output,
    delegate_seat,
    delegate_shm,
    delegate_xdg_shell,
};

use smithay::backend::renderer::utils::on_commit_buffer_handler;

use smithay::desktop::{PopupKind, Window};

use smithay::wayland::{
    buffer::BufferHandler,
    compositor::{
        CompositorClientState,
        CompositorHandler,
        CompositorState,
    },
    output::OutputHandler,
    shell::xdg::{
        PopupSurface,
        PositionerState,
        ToplevelSurface,
        XdgShellHandler,
        XdgShellState,
    },
    shm::{ShmHandler, ShmState},
};

use crate::surface::{next_window_position, window_for_surface};

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
        state.states.set(
            smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Activated
        );
    });

    surface.send_configure();

    let window = Window::new_wayland_window(surface);
    let position = next_window_position(&self.space);

    self.space.map_element(window, position, true);
}

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        println!("MITOS GUI: application window closed");

        if let Some(window) = window_for_surface(&self.space, surface.wl_surface()) {
            self.space.unmap_elem(&window);
        }
    }

    fn new_popup(
        &mut self,
        surface: PopupSurface,
        _positioner: PositionerState,
    ) {
        println!("MITOS GUI: new popup");

        if let Err(err) = self.popups.track_popup(PopupKind::Xdg(surface)) {
            tracing::warn!(?err, "MITOS GUI: failed to track popup");
        }
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

        // Keep the window's tracked bounding box in sync with whatever
        // buffer it just committed. Without this, Space's idea of a
        // window's size goes stale the moment a client resizes.
        if let Some(window) = window_for_surface(&self.space, surface) {
            window.on_commit();
        }

        self.popups.commit(surface);
    }
}


// ============================================================
// OUTPUT
// ============================================================

impl OutputHandler for MitosGuiState {}


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
delegate_output!(MitosGuiState);
delegate_seat!(MitosGuiState);
