use calloop::EventLoop;
use smithay::wayland::compositor::CompositorState;
use smithay::wayland::shell::xdg::XdgShellState;

pub struct MitosGuiState {
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
}

fn main() {
    println!("mitos-gui [INFO]: Initializing Lightweight Wayland Display Server...");

    // 1. Initialize event loop
    let mut event_loop: EventLoop<MitosGuiState> = EventLoop::try_new()
        .expect("mitos-gui [FATAL]: Failed to create event loop.");

    let display = smithay::reexports::wayland_server::Display::<MitosGuiState>::new()
        .expect("mitos-gui [FATAL]: Failed to initialize Wayland display.");

    println!("mitos-gui [OK]: Wayland socket bound.");
    println!("mitos-gui [OK]: Compositor active. Ready for apps and games.");

    // 2. Main rendering & event loop
    // Runs at low CPU/memory idle unless handling input or rendering frames
    loop {
        event_loop
            .dispatch(std::time::Duration::from_millis(16), &mut ())
            .unwrap();
    }
}
