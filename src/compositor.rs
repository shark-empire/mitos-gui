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
        wl_output,
        wl_seat,
        wl_surface::WlSurface,
    },
    Client,
};

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::utils::Serial;

use crate::state::MitosGuiState;

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

        let window = Window::new_wayland_window(surface);
                
        // Assign to current workspace
        crate::wm::meta(&window).workspace = self.current_workspace;
        let position = next_window_position(&self.space);

        self.space.map_element(window.clone(), position, true);

        // Stage 4: Automatically focus new windows
        crate::wm::set_focus(self, Some(window));
        
        // Update dock running indicators
        crate::shell_interaction::update_running_state(self);
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

    // --------------------------------------------------------
    // XDG Toplevel Requests (Client-driven state changes)
    // --------------------------------------------------------

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = window_for_surface(&self.space, surface.wl_surface()) {
            crate::wm::request_maximize(self, &window);
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = window_for_surface(&self.space, surface.wl_surface()) {
            crate::wm::request_unmaximize(self, &window);
        }
    }

    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        _output: Option<wl_output::WlOutput>,
    ) {
        if let Some(window) = window_for_surface(&self.space, surface.wl_surface()) {
            crate::wm::request_fullscreen(self, &window);
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = window_for_surface(&self.space, surface.wl_surface()) {
            crate::wm::request_unfullscreen(self, &window);
        }
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = window_for_surface(&self.space, surface.wl_surface()) {
            crate::wm::request_minimize(self, &window);
        }
    }

    fn move_request(&mut self, surface: ToplevelSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        if let Some(window) = window_for_surface(&self.space, surface.wl_surface()) {
            crate::wm::begin_move(self, window);
        }
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        _seat: wl_seat::WlSeat,
        _serial: Serial,
        _edges: xdg_toplevel::ResizeEdge,
    ) {
        if let Some(window) = window_for_surface(&self.space, surface.wl_surface()) {
            crate::wm::begin_resize(self, window);
        }
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
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

// ============================================================
// COMPOSITOR
// ============================================================

impl CompositorHandler for MitosGuiState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<MitosClientState>().unwrap().compositor_state
    }


    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);

        if let Some(window) = window_for_surface(&self.space, surface) {
            window.on_commit();
            
            // Tell the main loop that a client updated its buffer
            self.pending_full_redraw = true; 
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

    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {
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
