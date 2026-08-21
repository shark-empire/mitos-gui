mod compositor;
mod state;
mod theme;

use calloop::EventLoop;

use smithay::input::keyboard::XkbConfig;
use smithay::input::SeatState;

use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::utils::Transform;

use smithay::reexports::wayland_server::Display;
use smithay::reexports::wayland_server::ListeningSocket;

use smithay::wayland::{
    compositor::CompositorState,
    shell::xdg::XdgShellState,
    shm::ShmState,
};

use state::MitosGuiState;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("MITOS GUI");
    println!("==========");
    println!("Initializing Wayland compositor...");

    tracing_subscriber::fmt::init();

    let mut event_loop: EventLoop<MitosGuiState> = EventLoop::try_new()?;

    let mut display: Display<MitosGuiState> = Display::new()?;

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

    let compositor_state = CompositorState::new::<MitosGuiState>(&display_handle);

    // --------------------------------------------------------
    // Shared memory buffers
    // --------------------------------------------------------

    let shm_state = ShmState::new::<MitosGuiState>(&display_handle, vec![]);

    // --------------------------------------------------------
    // XDG shell
    // --------------------------------------------------------

    let xdg_shell_state = XdgShellState::new::<MitosGuiState>(&display_handle);

    // --------------------------------------------------------
    // Output
    //
    // Virtual output until Stage 5 wires real DRM/KMS hardware.
    // Advertising at least one wl_output is required for most
    // real clients (GTK, Qt, etc.) to map a surface at all.
    // --------------------------------------------------------

    let output = Output::new(
        "MITOS-0".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "MITOS".into(),
            model: "Virtual".into(),
        },
    );

    let _output_global = output.create_global::<MitosGuiState>(&display_handle);

    let mode = Mode {
        size: (1920, 1080).into(),
        refresh: 60_000,
    };

    output.change_current_state(
        Some(mode),
        Some(Transform::Normal),
        None,
        Some((0, 0).into()),
    );

    output.set_preferred(mode);

    println!("MITOS GUI: output MITOS-0 registered (1920x1080@60)");

    // --------------------------------------------------------
    // Seat (keyboard + pointer capabilities)
    // --------------------------------------------------------

    let mut seat_state = SeatState::<MitosGuiState>::new();
    let seat = seat_state.new_wl_seat(&display_handle, "seat-0");

    seat.add_keyboard(XkbConfig::default(), 200, 25)?;
    seat.add_pointer();

    println!("MITOS GUI: seat-0 registered (keyboard + pointer)");

    // --------------------------------------------------------
    // MITOS state
    // --------------------------------------------------------

    let mut state = MitosGuiState::new(
        compositor_state,
        xdg_shell_state,
        shm_state,
        seat_state,
        seat,
        output,
    );

    println!("MITOS GUI: compositor initialized");
    println!("MITOS GUI: XDG shell initialized");
    println!("MITOS GUI: shared memory initialized");
    println!("MITOS GUI: event loop running");

    loop {
        event_loop.dispatch(
            std::time::Duration::from_millis(16),
            &mut state,
        )?;

        if let Some(stream) = listening_socket.accept()? {
            display.handle().insert_client(
                stream,
                std::sync::Arc::new(compositor::MitosClientState::default()),
            )?;
        }

        display.dispatch_clients(&mut state)?;
        display.flush_clients()?;
    }
}
