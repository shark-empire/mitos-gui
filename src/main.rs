mod compositor;
mod state;
mod theme;

use calloop::EventLoop;

use smithay::input::SeatState;

use smithay::reexports::wayland_server::Display;
use smithay::reexports::wayland_server::ListeningSocket;

use smithay::wayland::{
    compositor::CompositorState,
    shm::ShmState,
    shell::xdg::XdgShellState,
};

use state::MitosGuiState;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("MITOS GUI");
    println!("==========");
    println!("Initializing Wayland compositor...");

    tracing_subscriber::fmt::init();

    let mut event_loop: EventLoop<MitosGuiState> =
        EventLoop::try_new()?;

    let mut display: Display<MitosGuiState> =
        Display::new()?;

    let listening_socket = ListeningSocket::bind_auto("wayland", 0..10)
    .expect("Failed to create Wayland listening socket");

println!(
    "MITOS GUI: Wayland socket created at {:?}",
    listening_socket.socket_name()
);

    let display_handle = display.handle();

    // --------------------------------------------------------
    // Wayland compositor
    // --------------------------------------------------------

    let compositor_state =
        CompositorState::new::<MitosGuiState>(
            &display_handle,
        );

    // --------------------------------------------------------
    // Shared memory buffers
    // --------------------------------------------------------

    let shm_state =
        ShmState::new::<MitosGuiState>(
            &display_handle,
            vec![],
        );

    // --------------------------------------------------------
    // XDG shell
    // --------------------------------------------------------

    let xdg_shell_state =
        XdgShellState::new::<MitosGuiState>(
            &display_handle,
        );

    // --------------------------------------------------------
    // Seat
    // --------------------------------------------------------

    let seat_state =
        SeatState::<MitosGuiState>::new();

    // --------------------------------------------------------
    // MITOS state
    // --------------------------------------------------------

    let mut state = MitosGuiState::new(
        compositor_state,
        xdg_shell_state,
        shm_state,
        seat_state,
    );

    println!("MITOS GUI: compositor initialized");
    println!("MITOS GUI: XDG shell initialized");
    println!("MITOS GUI: shared memory initialized");
    println!("MITOS GUI: seat initialized");
    println!("MITOS GUI: event loop running");

loop {
    event_loop.dispatch(
        std::time::Duration::from_millis(16),
        &mut state,
    )?;

    if let Some(stream) = listening_socket.accept()? {
        display
            .handle()
            .insert_client(
                stream,
                std::sync::Arc::new(
                    compositor::MitosClientState::default()
                ),
            )?;
    }

    display.dispatch_clients(&mut state)?;
    display.flush_clients()?;
}

}
