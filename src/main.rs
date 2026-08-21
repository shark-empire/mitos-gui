mod compositor;
mod state;
mod theme;

use calloop::EventLoop;
use smithay::wayland::compositor::CompositorState;
use smithay::wayland::shell::xdg::XdgShellState;

use state::MitosGuiState;

fn main() {
    println!("MITOS GUI");
    println!("==========");
    println!("Initializing Wayland compositor...");

    tracing_subscriber::fmt::init();

    let mut event_loop: EventLoop<MitosGuiState> =
        EventLoop::try_new()
            .expect("MITOS GUI: failed to create event loop");

    let display =
        smithay::reexports::wayland_server::Display::<MitosGuiState>::new()
            .expect("MITOS GUI: failed to create Wayland display");

    let display_handle = display.handle();

    let compositor_state =
        CompositorState::new::<MitosGuiState>(
            &display_handle,
            6,
        );

    let xdg_shell_state =
        XdgShellState::new::<MitosGuiState>(
            &display_handle,
        );

    let _state = MitosGuiState::new(
        compositor_state,
        xdg_shell_state,
    );

    println!("MITOS GUI: compositor initialized");
    println!("MITOS GUI: theme initialized");
    println!("MITOS GUI: waiting for clients...");

    loop {
        event_loop
            .dispatch(
                std::time::Duration::from_millis(16),
                &mut _state,
            )
            .expect("MITOS GUI: event loop failed");
    }
}
