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

use smithay::backend::renderer::damage::{
    Error as DamageTrackerError,
    OutputDamageTracker,
};
use smithay::backend::renderer::element::solid::SolidColorBuffer;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::ImportMemWl;
use smithay::backend::winit::{self, WinitEvent};
use smithay::backend::SwapBuffersError;

use smithay::input::keyboard::XkbConfig;
use smithay::input::SeatState;

use smithay::output::{
    Mode,
    Output,
    PhysicalProperties,
    Subpixel,
};

use smithay::utils::{
    Scale,
    Transform,
};

use smithay::reexports::wayland_server::{
    Display,
    ListeningSocket,
};

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

    let mut event_loop: EventLoop<MitosGuiState> =
        EventLoop::try_new()?;

    let mut display: Display<MitosGuiState> =
        Display::new()?;

    let listening_socket =
        ListeningSocket::bind_auto("wayland", 0..10)
            .expect("Failed to create Wayland listening socket");

    println!(
        "MITOS GUI: Wayland socket created at {:?}",
        listening_socket.socket_name()
    );

    let display_handle = display.handle();

    // ============================================================
    // GPU BACKEND
    // ============================================================

    let (mut backend, mut winit_event_loop) =
        winit::init::<GlesRenderer>()?;

    println!(
        "MITOS GUI: GLES renderer initialized (winit backend)"
    );

    // ============================================================
    // WAYLAND COMPOSITOR
    // ============================================================

    let compositor_state =
        CompositorState::new::<MitosGuiState>(&display_handle);

    // ============================================================
    // SHM
    // ============================================================

    let mut shm_state =
        ShmState::new::<MitosGuiState>(
            &display_handle,
            vec![],
        );

    shm_state.update_formats(
        backend.renderer().shm_formats()
    );

    // ============================================================
    // XDG SHELL
    // ============================================================

    let xdg_shell_state =
        XdgShellState::new::<MitosGuiState>(
            &display_handle
        );

    // ============================================================
    // OUTPUT
    // ============================================================

    let output = Output::new(
        OUTPUT_NAME.to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "MITOS".into(),
            model: "Virtual".into(),
        },
    );

    let _output_global =
        output.create_global::<MitosGuiState>(
            &display_handle
        );

    let mode = Mode {
        size: backend.window_size(),
        refresh: 60_000,
    };

    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        None,
        Some((0, 0).into()),
    );

    output.set_preferred(mode);

    println!(
        "MITOS GUI: output {OUTPUT_NAME} registered ({}x{}@60)",
        mode.size.w,
        mode.size.h
    );

    // ============================================================
    // SEAT
    // ============================================================

    let mut seat_state =
        SeatState::<MitosGuiState>::new();

    let mut seat =
        seat_state.new_wl_seat(
            &display_handle,
            "seat-0",
        );

    seat.add_keyboard(
        XkbConfig::default(),
        200,
        25,
    )?;

    seat.add_pointer();

    println!(
        "MITOS GUI: seat-0 registered (keyboard + pointer)"
    );

    // ============================================================
    // HOME SCREEN
    // ============================================================

    let home_screen =
        desktop::HomeScreenConfig::load();

    // ============================================================
    // MITOS STATE
    // ============================================================

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

    // ============================================================
    // INITIAL SHELL LAYOUT
    //
    // Stage 3:
    //
    // HomeScreenConfig
    //        ↓
    // ShellLayout::calculate()
    //        ↓
    // MitosShell
    // ============================================================

    if let Some(output_geometry) =
        state.space.output_geometry(&output)
    {
        let layout =
            desktop::ShellLayout::calculate(
                &state.home_screen,
                output_geometry.size,
            );

        state.shell.top_bar = layout.top_bar;
        state.shell.launcher = layout.launcher;
        state.shell.dock = layout.dock;

    }

    // ============================================================
    // DAMAGE TRACKING
    // ============================================================

    let mut damage_tracker =
        OutputDamageTracker::from_output(&output);

    // ============================================================
    // TOP BAR GPU BUFFERS
    // ============================================================

    let mut top_bar_shadow_buffer =
        SolidColorBuffer::new(
            (0, 0),
            renderer::shadow_color(),
        );

    let mut top_bar_highlight_buffer =
        SolidColorBuffer::new(
            (0, 0),
            renderer::glass_highlight_color(),
        );

    let mut top_bar_border_buffer =
        SolidColorBuffer::new(
            (0, 0),
            {
                let c = theme::MitosTheme::BORDER;

                smithay::backend::renderer::Color32F::new(
                    c.r,
                    c.g,
                    c.b,
                    c.a,
                )
            },
        );

    let mut top_bar_glass =
    renderer::create_glass_panel_element(
        backend.renderer()
    )?;

let mut launcher_glass =
    renderer::create_glass_panel_element(
        backend.renderer()
    )?;

let mut dock_glass =
    renderer::create_glass_panel_element(
        backend.renderer()
    )?;

    // ============================================================
    // FULL REDRAW
    // ============================================================

    let mut full_redraw_frames: u8 = 4;

    println!("MITOS GUI: compositor initialized");
    println!("MITOS GUI: XDG shell initialized");
    println!("MITOS GUI: shared memory initialized");
    println!("MITOS GUI: window space initialized");
    println!("MITOS GUI: Stage 3 shell initialized");
    println!("MITOS GUI: event loop running");

    // ============================================================
    // MAIN LOOP
    // ============================================================

    loop {
        // ========================================================
        // INPUT / WINDOW EVENTS
        // ========================================================

        let pump_status =
            winit_event_loop.dispatch_new_events(
                |event| match event {
                    WinitEvent::Resized { size, .. } => {
                        let mode = Mode {
                            size,
                            refresh: 60_000,
                        };

                        output.change_current_state(
                            Some(mode),
                            None,
                            None,
                            None,
                        );

                        output.set_preferred(mode);

                        damage_tracker =
                            OutputDamageTracker::from_output(
                                &output
                            );

                        full_redraw_frames = 4;

                        // ------------------------------------------------
                        // Recalculate Stage 3 shell geometry.
                        // ------------------------------------------------

                        if let Some(output_geometry) =
                            state.space.output_geometry(&output)
                        {
                            let layout =
                                desktop::ShellLayout::calculate(
                                    &state.home_screen,
                                    output_geometry.size,
                                );

                            state.shell.top_bar =
                                layout.top_bar;

                            state.shell.launcher =
                                layout.launcher;

                            state.shell.dock =
                                layout.dock;

        
                        }
                    }

                    WinitEvent::Input(event) => {
                        input::process_input_event(
                            &mut state,
                            &output,
                            event,
                        );
                    }

                    _ => {}
                },
            );

        if let PumpStatus::Exit(_) = pump_status {
            println!(
                "MITOS GUI: window closed, shutting down"
            );

            break;
        }

        // ========================================================
        // WAYLAND CLIENT CONNECTION
        // ========================================================

        if let Some(stream) =
            listening_socket.accept()?
        {
            display.handle().insert_client(
                stream,
                Arc::new(
                    compositor::MitosClientState::default()
                ),
            )?;
        }

        // ========================================================
        // RENDER
        // ========================================================

        let age = if full_redraw_frames > 0 {
            0
        } else {
            backend.buffer_age().unwrap_or(0)
        };

        let scale =
            Scale::from(
                output.current_scale().fractional_scale()
            );

        // ========================================================
        // TOP BAR GPU RESOURCES
        // ========================================================

        if let Some(panel) =
            state.shell.top_bar
        {
            let width = panel.size.0;
            let height = panel.size.1;

            // Shadow.
            top_bar_shadow_buffer.update(
                (width, 8),
                renderer::shadow_color(),
            );

            // Highlight.
            top_bar_highlight_buffer.update(
                (width, 2),
                renderer::glass_highlight_color(),
            );

            // Border.
            let border =
                theme::MitosTheme::BORDER;

            top_bar_border_buffer.update(
                (width, 1),
                smithay::backend::renderer::Color32F::new(
                    border.r,
                    border.g,
                    border.b,
                    border.a,
                ),
            );

            // Keep renderer-facing panel synchronized.
            state.shell.top_bar = Some(panel);

            let _ = height;
        } else {
           state.shell.top_bar = None;
        }

        // ========================================================
        // DRAW FRAME
        // ========================================================

        let render_result =
            backend.bind().and_then(
                |(renderer, mut framebuffer)| {
        let shell_elements =
      renderer::collect_shell_elements(
        renderer,
        &state.shell,
        &mut top_bar_glass,
        &mut launcher_glass,
        &mut dock_glass,
        &top_bar_shadow_buffer,
        &top_bar_highlight_buffer,
        &top_bar_border_buffer,
        scale,
    );

let elements =
    renderer::collect_frame_elements(
        renderer,
        &state.space,
        scale,
        shell_elements,
    );

                    damage_tracker
                        .render_output(
                            renderer,
                            &mut framebuffer,
                            age,
                            &elements,
                            renderer::clear_color(
                                &state.home_screen
                            ),
                        )
                        .map_err(|err| match err {
                            DamageTrackerError::Rendering(
                                err
                            ) => err.into(),

                            _ => unreachable!(
                                "output-mode errors can't happen: mode is always set above"
                            ),
                        })
                },
            );

        // ========================================================
        // SUBMIT FRAME
        // ========================================================

        match render_result {
            Ok(render_output_result) => {
                if let Some(damage) =
                    render_output_result.damage
                {
                    if let Err(err) =
                        backend.submit(Some(damage))
                    {
                        tracing::warn!(
                            "MITOS GUI: failed to submit frame: {err}"
                        );
                    }
                }

                full_redraw_frames =
                    full_redraw_frames.saturating_sub(1);

                // ------------------------------------------------
                // Frame callbacks.
                // ------------------------------------------------

                let now = state.clock.now();

                for window in state.space.elements() {
                    window.send_frame(
                        &output,
                        now,
                        Some(Duration::from_secs(1)),
                        |_, _| Some(output.clone()),
                    );
                }
            }

            Err(
                SwapBuffersError::ContextLost(err)
            ) => {
                tracing::error!(
                    "MITOS GUI: critical rendering error, exiting: {err}"
                );

                break;
            }

            Err(err) => {
                tracing::warn!(
                    "MITOS GUI: render error: {err}"
                );
            }
        }

        // ========================================================
        // MAINTENANCE
        // ========================================================

        state.space.refresh();
        state.popups.cleanup();

        display.dispatch_clients(&mut state)?;
        display.flush_clients()?;

        event_loop.dispatch(
            Duration::from_millis(1),
            &mut state,
        )?;
    }

    Ok(())
}
