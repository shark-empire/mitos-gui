use calloop::EventLoop;
use smithay::wayland::compositor::CompositorState;
use smithay::wayland::shell::xdg::XdgShellState;

pub struct MitosGuiState {
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
}

fn main() {
    println!("MITOS GUI");
    println!("Initializing graphical environment...");

    let mut event_loop: EventLoop<MitosGuiState> =
        EventLoop::try_new()
            .expect("MITOS GUI: failed to create event loop");

    let _display =
        smithay::reexports::wayland_server::Display::<MitosGuiState>::new()
            .expect("MITOS GUI: failed to create Wayland display");

    println!("MITOS GUI: display initialized");
    println!("MITOS GUI: compositor ready");

    /*
     * Temporary compositor state.
     *
     * The full Smithay compositor will be initialized here as
     * we add rendering, surfaces, input, windows, and effects.
     */

    loop {
        /*
         * The event loop requires the actual MitosGuiState.
         *
         * This placeholder will be replaced when the compositor
         * state is fully initialized.
         */
        std::thread::sleep(
            std::time::Duration::from_millis(16)
        );

        let _ = &mut event_loop;
    }
}
