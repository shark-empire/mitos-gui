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

        _surface: PopupSurface,        &mut self.xdg_shell_state
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

        self.space.map_element(window.clone(), position, true);

        // Stage 4: Automatically focus new windows
        crate::wm::set_focus(self, Some(window));
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        println!("MITOS GUI: application window closed");

        // Stage 4: Clean up WM state before unmapping
        crate::wm::cleanup_destroyed(self, surface.wl_surface());

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
        _seat
        _seat: wl_seat::: wl_seat::WlSeat,
        _serialWlSeat,
        _serial: Serial,
    ) {
: Serial,
    ) {
        println!("MIT        println!("MITOS GUI: popupOS GUI: popup grab");
    grab");
    }

    fn }

    fn reposition_request(
        &mut reposition_request(
        &mut self,
        _surface: Popup self,
        _surface: PopupSurface,
        _positioner:Surface,
        _positioner: PositionerState,
        _token PositionerState,
        _token: u32,
    ): u32,
    ) {
        println!("MITOS GUI {
        println!("MITOS GUI: popup reposition request");
   : popup reposition request");
    }
}


// ============================================================
 }
}


// ============================================================
// SHM
// ============================================================

// SHM
// ============================================================

impl ShmHandlerimpl ShmHandler for MitosGui for MitosGuiState {
    fn shm_state(&State {
    fn shm_state(&self) -> &self) -> &ShmState {ShmState {
        &self
        &self.shm_state
.shm_state
    }
}


// ============================================================    }
}


// ============================================================
// BUFFER
// ============================================================


// BUFFER
// ============================================================

impl BufferHandler for MitosGuiStateimpl BufferHandler for MitosGuiState {
    fn {
    fn buffer_destroyed( buffer_destroyed(
        &mut self,
       
        &mut self,
        _buffer: & _buffer: &wl_buffer::Wwl_buffer::WlBuffer,
lBuffer,
    ) {
    ) {
    }
}


// ============================================================    }
}


// ============================================================
// COMPOSITOR
// =================================================
// COMPOSITOR
// ============================================================

impl Com===========

impl CompositorHandler for MitpositorHandler for MitosGuiState {osGuiState {
    fn compositor
    fn compositor_state(&mut self_state(&mut self) -> &mut) -> &mut CompositorState { CompositorState {
        &mut
        &mut self.compositor_state
    }

 self.compositor_state
    }

    fn client_com    fn client_compositor_state<'apositor_state<'a>(
        &>(
        &self,
       self,
        client: &'a client: &'a Client,
    Client,
    ) -> &'a ) -> &'a CompositorClientState CompositorClientState {
        & {
        &client
            .client
            .get_data::<Mget_data::<MitosClientState>()itosClientState>()
            .expect
            .expect("MITOS GUI("MITOS GUI: missing client state")
            .: missing client state")
            .compositor_state
    }

   compositor_state
    }

    fn commit(&mut fn commit(&mut self, surface: self, surface: &WlSurface) {
        &WlSurface) {
        on_commit_buffer_handler on_commit_buffer_handler::<Self>(surface::<Self>(surface);

        //);

        // Keep the window's Keep the window's tracked bounding box in tracked bounding box in sync with whatever
 sync with whatever
        // buffer it        // buffer it just committed. Without just committed. Without this, Space's idea of a
 this, Space's idea of a
        // window's        // window's size goes stale the size goes stale the moment a client res moment a client resizes.
       izes.
        if let Some(window if let Some(window) = window_for) = window_for_surface(&self.space_surface(&self.space, surface) {, surface) {
            window.on
            window.on_commit();
       _commit();
        }

        self }

        self.popups.commit(surface.popups.commit(surface);
    }
}


//);
    }
}


// ============================================================
// OUTPUT
// ================================================= ============================================================
// OUTPUT
// ============================================================

impl OutputHandler for Mitos===========

impl OutputHandler for MitosGuiState {}


// ============================================================
GuiState {}


// ============================================================
// CLIENT STATE
// ============================================================

// CLIENT STATE
// ============================================================

#[derive(Default)]
pub struct Mit#[derive(Default)]
pub struct MitosClientState {
    pub compositorosClientState {
    pub compositor_state: CompositorClientState,
_state: CompositorClientState,
}

impl ClientData for Mitos}

impl ClientData for MitosClientState {
    fn initialized(&ClientState {
    fn initialized(&self, _client_id: ClientIdself, _client_id: ClientId) {
        println!("MITOS) {
        println!("MITOS GUI: Wayland GUI: Wayland client connected");
 client connected");
    }

       }

    fn disconnected(
 fn disconnected(
        &self,        &self,
        _client
        _client_id: ClientId,
        __id: ClientId,
        _reason: DisconnectReason,
    )reason: DisconnectReason,
    ) {
        println!("MITOS GUI {
        println!("MITOS GUI: Wayland client disconnected");
   : Wayland client disconnected");
    }
}

 }
}

// ============================================================
// ============================================================
// SMITHAY// SMITHAY DELEGATES
 DELEGATES
// ============================================================

// ============================================================

delegate_xdg_shelldelegate_xdg_shell!(MitosGuiState);
delegate!(MitosGuiState);
delegate_compositor!(MitosGuiState);_compositor!(MitosGuiState);
delegate_shm
delegate_shm!(MitosGui!(MitosGuiState);
delegateState);
delegate_output!(Mitos_output!(MitosGuiState);
GuiState);
delegate_seat!(Mdelegate_seat!(MitosGuiState);itosGuiState);
