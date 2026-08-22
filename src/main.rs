mod animation;
mod compositor;
mod desktop;
mod input;
mod keyboard;
mod pointer;
mod renderer;
mod state;
mod surface;
mod theme;

use std::sync::Arc;
use std::time::Duration;

use calloop::EventLoop;

use smithay::backend::renderer::damage::{Error as DamageTrackerError, OutputDamageTracker};
use smithay::backend::renderer::element::solid::SolidColorBuffer;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::ImportMemWl;
use smithay::backend::winit::{self, WinitEvent};
use smithay::backend::SwapBuffersError;

use smithay::input::keyboard::XkbConfig;
use smithay::input::SeatState;

use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::utils::{Scale, Transform};

use smithay::reexports::wayland_server::Display;
use smithay::reexports::wayland_server::ListeningSocket;
use smithay::reexports::winit::platform::pump_events::PumpStatus;

use smithay::wayland::{
    compositor::CompositorState,
    shell::xdg::XdgShellState,
    shm::ShmState,
};

use state::MitosGuiState;

const OUTPUT_NAME: &str = "MITOS-0";

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
    // GPU backend
    //
    // winit stands in for real display hardware until Stage 5 wires up
    // DRM/GBM: it hands us a real GLES context plus a host window to
    // present into, and doubles as our input source (forwarding host
    // keyboard/mouse events) in the meantime. Nothing downstream of
    // this call -- the renderer, the damage tracker, input.rs -- knows
    // or cares that it isn't real hardware yet.
    // --------------------------------------------------------

    let (mut backend, mut winit_event_loop) = winit::init::<GlesRenderer>()?;

    println!("MITOS GUI: GLES renderer initialized (winit backend)");

    // --------------------------------------------------------
    // Wayland compositor
    // --------------------------------------------------------

    let compositor_state = CompositorState::new::<MitosGuiState>(&display_handle);

    // --------------------------------------------------------
    // Shared memory buffers
    //
    // Seeded with whatever pixel formats the GLES renderer actually
    // supports, on top of the ARGB8888/XRGB8888 pair `ShmState::new`
    // always advertises regardless.
    // --------------------------------------------------------

    let mut shm_state = ShmState::new::<MitosGuiState>(&display_handle, vec![]);
    shm_state.update_formats(backend.renderer().shm_formats());

    // --------------------------------------------------------
    // XDG shell
    // --------------------------------------------------------

    let xdg_shell_state = XdgShellState::new::<MitosGuiState>(&display_handle);

    // --------------------------------------------------------
    // Output
    //
    // Sized to match whatever window winit actually opened, rather
    // than a hardcoded 1920x1080 — Stage 5's real DRM output will
    // report its own mode the same way, through the same `Output`.
    // --------------------------------------------------------

    let output = Output::new(
        OUTPUT_NAME.to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "MITOS".into(),
            model: "Virtual".into(),
        },
    );

    let _output_global = output.create_global::<MitosGuiState>(&display_handle);

    let mode = Mode {
        size: backend.window_size(),
        refresh: 60_000,
    };

    // GLES renders into a framebuffer that's vertically flipped
    // relative to winit's window; telling the output it's
    // `Flipped180` keeps the coordinate space damage tracking works in
    // consistent with what winit actually presents on screen.
    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        None,
        Some((0, 0).into()),
    );

    output.set_preferred(mode);

    println!(
        "MITOS GUI: output {OUTPUT_NAME} registered ({}x{}@60)",
        mode.size.w, mode.size.h
    );

    // --------------------------------------------------------
    // Seat (keyboard + pointer capabilities)
    // --------------------------------------------------------

    let mut seat_state = SeatState::<MitosGuiState>::new();
    let mut seat = seat_state.new_wl_seat(&display_handle, "seat-0");

    seat.add_keyboard(XkbConfig::default(), 200, 25)?;
    seat.add_pointer();

    println!("MITOS GUI: seat-0 registered (keyboard + pointer)");

    // --------------------------------------------------------
    // Home screen configuration (Stage 3)
    //
    // Loaded once, up front -- clear_color() reads the resolved
    // struct every frame, not the config file.
    // --------------------------------------------------------

    let home_screen = desktop::HomeScreenConfig::load();

    // --------------------------------------------------------
    // MITOS state
    // --------------------------------------------------------

    let mut state = MitosGuiState::new(
        compositor_state,
        xdg_shell_state,
        shm_state,
        seat_state,
        seat,
        output.clone(),
        home_screen,
        None,
    );

    // Damage tracking: rebuilt from scratch on resize (see the
    // `WinitEvent::Resized` arm below), since a resized output
    // invalidates whatever damage state was tracked against its old
    // dimensions.
    let mut damage_tracker = OutputDamageTracker::from_output(&output);

    // The top bar's pixels, persisted across frames rather than
    // rebuilt each time -- `SolidColorBuffer::update` no-ops when the
    // size and color haven't changed, so a motionless bar costs the
    // damage tracker nothing after the frame it first appears in.
    // Real size follows below, once `state.space` knows the output's
    // logical geometry; (0, 0) here just avoids a second Option layer
    // before that first update runs.
    let mut top_bar_buffer = SolidColorBuffer::new((0, 0), renderer::top_bar_color());

    // Forces a handful of full-framebuffer redraws right after
    // anything that invalidates the backbuffer's contents wholesale
    // (startup, resize) instead of trusting possibly-stale damage.
    let mut full_redraw_frames: u8 = 0;

    println!("MITOS GUI: compositor initialized");
    println!("MITOS GUI: XDG shell initialized");
    println!("MITOS GUI: shared memory initialized");
    println!("MITOS GUI: window space initialized");
    println!("MITOS GUI: event loop running");

    loop {
        // --------------------------------------------------------
        // Input
        //
        // winit's pump delivers host keyboard/mouse events plus resize
        // notifications for the window it owns, non-blocking.
        // --------------------------------------------------------

        let pump_status = winit_event_loop.dispatch_new_events(|event| match event {
            WinitEvent::Resized { size, .. } => {
                let mode = Mode {
                    size,
                    refresh: 60_000,
                };
                output.change_current_state(Some(mode), None, None, None);
                output.set_preferred(mode);

                damage_tracker = OutputDamageTracker::from_output(&output);
                full_redraw_frames = 4;
            }
            WinitEvent::Input(event) => {
                input::process_input_event(&mut state, &output, event);
            }
            _ => {}
        });

        if let PumpStatus::Exit(_) = pump_status {
            println!("MITOS GUI: window closed, shutting down");
            break;
        }

        if let Some(stream) = listening_socket.accept()? {
            display.handle().insert_client(
                stream,
                Arc::new(compositor::MitosClientState::default()),
            )?;
        }

        // --------------------------------------------------------
        // Render
        // --------------------------------------------------------

        let age = if full_redraw_frames > 0 {
            0
        } else {
            backend.buffer_age().unwrap_or(0)
        };

        let scale = Scale::from(output.current_scale().fractional_scale());

        // Keep the top bar's buffer sized to the output's current
        // logical width -- `output_geometry` already resolves mode and
        // scale into logical pixels, so there's no manual physical/scale
        // math to get wrong here. `update` is a no-op if nothing
        // actually changed since last frame.
if state.home_screen.top_bar {
    if let Some(output_geometry) = state.space.output_geometry(&output) {
        let width = output_geometry.size.w;
        let height = state
            .home_screen
            .top_bar_height
            .max(1.0)
            .round() as i32;

        let panel = renderer::GlassPanel::top_bar(width, height);

        top_bar_buffer.update(
            panel.size,
            panel.tint,
        );

        state.top_bar_panel = Some(panel);
    }
} else {
    state.top_bar_panel = None;
}

       let top_bar = if state.top_bar_panel.is_some() {
    Some(&top_bar_buffer)
} else {
    None
};

        let render_result = backend.bind().and_then(|(renderer, mut framebuffer)| {
            let elements =
                renderer::collect_frame_elements(renderer, &state.space, scale, top_bar);

            damage_tracker
                .render_output(
                    renderer,
                    &mut framebuffer,
                    age,
                    &elements,
                    renderer::clear_color(&state.home_screen),
                )
                .map_err(|err| match err {
                    DamageTrackerError::Rendering(err) => err.into(),
                    _ => unreachable!("output-mode errors can't happen: mode is always set above"),
                })
        });

        match render_result {
            Ok(render_output_result) => {
                if let Some(damage) = render_output_result.damage {
                    if let Err(err) = backend.submit(Some(damage)) {
                        tracing::warn!("MITOS GUI: failed to submit frame: {err}");
                    }
                }

                full_redraw_frames = full_redraw_frames.saturating_sub(1);

                // Frame scheduling: tell every mapped client its last
                // frame was presented, so well-behaved clients throttle
                // redraws to this output's refresh rate instead of
                // rendering in a tight loop.
                let now = state.clock.now();
                for window in state.space.elements() {
                    window.send_frame(&output, now, Some(Duration::from_secs(1)), |_, _| {
                        Some(output.clone())
                    });
                }
            }
            Err(SwapBuffersError::ContextLost(err)) => {
                tracing::error!("MITOS GUI: critical rendering error, exiting: {err}");
                break;
            }
            Err(err) => {
                tracing::warn!("MITOS GUI: render error: {err}");
            }
        }

        // Reconcile output enter/leave state for mapped windows and
        // drop any popups whose parent surface is gone. Cheap, and
        // needs to happen before every flush to stay accurate.
        state.space.refresh();
        state.popups.cleanup();

        display.dispatch_clients(&mut state)?;
        display.flush_clients()?;

        event_loop.dispatch(Duration::from_millis(1), &mut state)?;
    }

    Ok(())
}
