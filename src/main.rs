mod animation;
mod compositor;
mod config_watcher;
mod desktop;
mod drm_backend;
mod input;
mod keyboard;
mod notify;
mod pointer;
mod renderer;
mod shell_interaction;
mod state;
mod surface;
mod text;
mod theme;
mod wm;
mod notifications;
mod status;
mod icons;
mod auth;
mod session;
mod gestures;

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

    // --------------------------------------------------------
    // Production backend selection.
    //
    // mitos-gui.service will use: ExecStart=/usr/bin/mitos-gui --drm
    // --------------------------------------------------------
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--drm") {
        // Production path:
        // libseat -> DRM -> GBM -> EGL -> vblank frame loop.
        return drm_backend::run_drm();
    }

    println!("Initializing Wayland compositor...");

    tracing_subscriber::fmt::init();

    // ============================================================
    // EVENT LOOP
    // ============================================================

    let mut event_loop: EventLoop<MitosGuiState> =
        EventLoop::try_new()?;

    // ============================================================
    // LIVE CONFIG RELOAD (inotify)
    // ============================================================

    let (config_tx, config_rx) =
        calloop::channel::channel::<config_watcher::ConfigChanged>();

    // Keep one sender alive so the channel never closes,
    // even if the watcher fails to start.
    let _config_keepalive = config_tx.clone();

    // IMPORTANT: bind to a named variable.
    // `let _ = ...` would drop the watcher immediately and kill inotify.
    let _config_watcher = config_watcher::ConfigWatcher::start(config_tx);

    event_loop
        .handle()
        .insert_source(config_rx, |event, _metadata, state| {
            if let calloop::channel::Event::Msg(_) = event {
                state.reload_configuration();
            }
        })
        .expect("MITOS GUI: failed to register config channel");

    // ============================================================
    // WAYLAND DISPLAY
    // ============================================================

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
    // WALLPAPER
    // ============================================================

    let mut wallpaper =
        renderer::Wallpaper::load_default()
            .map_err(|err| {
                format!(
                    "MITOS GUI: {err}"
                )
            })?;

    // ============================================================
    // WAYLAND COMPOSITOR
    // ============================================================

    let compositor_state =
        CompositorState::new::<MitosGuiState>(
            &display_handle,
        );

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
            &display_handle,
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
            &display_handle,
        );

    let mode = Mode {
        size: backend.window_size(),
        refresh: 60_000,
    };

    output.change_current_state(
        Some(mode),
        Some(Transform::Normal),
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

    // Apply initial runtime theme overrides
    theme::MitosTheme::apply_runtime(&home_screen);

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

    // ============================================================
    // DOCK GPU BUFFERS
    // ============================================================

    let mut dock_shadow_buffer =
        SolidColorBuffer::new(
            (0, 0),
            renderer::shadow_color(),
        );

    let mut dock_highlight_buffer =
        SolidColorBuffer::new(
            (0, 0),
            renderer::glass_highlight_color(),
        );

    let mut dock_border_buffer =
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

    // ============================================================
    // GLASS SHADERS
    //
    // One shader element per shell component.
    // ============================================================

    let mut top_bar_glass =
        renderer::create_glass_panel_element(
            backend.renderer(),
        )?;

    let mut launcher_glass =
        renderer::create_glass_panel_element(
            backend.renderer(),
        )?;

    let mut dock_glass = 
        renderer::create_glass_panel_element(
            backend.renderer()
        )?;
    let mut shell_text = renderer::ShellTextState::new();
    
    let mut tray = renderer::TrayState::new();

    
    // NEW: Window shadow/border cache
    let mut window_chrome = renderer::WindowChrome::new(); 


    // ============================================================
    // FULL REDRAW & READINESS FLAGS
    // ============================================================

    let mut full_redraw_frames: u8 = 4;
    let mut ready_sent = false;

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
                    // ------------------------------------------------
                    // WINDOW RESIZED
                    // ------------------------------------------------

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
                                &output,
                            );

                        full_redraw_frames = 4;

                        // ------------------------------------------------
                        // Recalculate MITOS shell geometry.
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

                    // ------------------------------------------------
                    // INPUT
                    // ------------------------------------------------

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
                    compositor::MitosClientState::default(),
                ),
            )?;
        }

        // ========================================================
        // LIVE CONFIG RELOAD
        // ========================================================

        if state.pending_full_redraw {
            state.pending_full_redraw = false;
            full_redraw_frames = 4;

            // Recompile the glass shaders with the new theme values.
            if let Ok(glass) =
                renderer::create_glass_panel_element(backend.renderer())
            {
                top_bar_glass = glass;
            }
            if let Ok(glass) =
                renderer::create_glass_panel_element(backend.renderer())
            {
                launcher_glass = glass;
            }
            if let Ok(glass) =
                renderer::create_glass_panel_element(backend.renderer())
            {
                dock_glass = glass;
            }

            // Swap wallpaper if home.conf points to a new one.
            if let Some(wp_path) = state.home_screen.wallpaper_path.clone() {
                match renderer::Wallpaper::load_from_path(&wp_path) {
                    Ok(wp) => wallpaper = wp,
                    Err(err) => tracing::warn!("MITOS GUI: {err}"),
                }
            }
        }

        // Re-rasterize clock / launcher text if it changed
        if shell_text.refresh(&state.shell) {
            state.pending_full_redraw = true;
        }
        
        if state.poll_status_if_due() {
            state.pending_full_redraw = true;
        }

        if tray.refresh(
            &state.network,
            &state.battery,
            state.volume,
            state.muted,
        ) {
            state.pending_full_redraw = true;
        }



      // Tick notifications (auto-dismiss expired ones)
        if state.notifications.tick() {
            state.pending_full_redraw = true;
        }

        // ========================================================
        // FRAME SCHEDULING & DAMAGE TRACKING
        // ========================================================
        // Only render if we have damage, a forced full redraw, or initial frames.
        let should_render = state.pending_full_redraw || full_redraw_frames > 0;

        if should_render {
            state.pending_full_redraw = false; // Consume any newly arrived damage

            // ========================================================
            // BUFFER AGE
            // ========================================================

            let age = if full_redraw_frames > 0 {
                0
            } else {
                backend.buffer_age().unwrap_or(0)
            };

            // ========================================================
            // OUTPUT SCALE
            // ========================================================

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

                // ----------------------------------------------------
                // Shadow
                // ----------------------------------------------------

                top_bar_shadow_buffer.update(
                    (width, 8),
                    renderer::shadow_color(),
                );

                // ----------------------------------------------------
                // Highlight
                // ----------------------------------------------------

                top_bar_highlight_buffer.update(
                    (width, 2),
                    renderer::glass_highlight_color(),
                );

                // ----------------------------------------------------
                // Border
                // ----------------------------------------------------

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
            }

            // ============================================================
            // DOCK GPU RESOURCES
            // ============================================================

            if let Some(panel) = state.shell.dock {
                let width = panel.size.0;

                dock_shadow_buffer.update(
                    (width, 12),
                    renderer::shadow_color(),
                );

                dock_highlight_buffer.update(
                    (width, 2),
                    renderer::glass_highlight_color(),
                );

                let border = theme::MitosTheme::BORDER;

                dock_border_buffer.update(
                    (width, 1),
                    smithay::backend::renderer::Color32F::new(
                        border.r,
                        border.g,
                        border.b,
                        border.a,
                    ),
                );
            }
            
            // ========================================================
            // DRAW FRAME
            // ========================================================

            let render_result =
                backend.bind().and_then(
                    |(renderer, mut framebuffer)| {
                        // ------------------------------------------------
                        // MITOS SHELL
                        // ------------------------------------------------
                        let shell_elements =
                            renderer::collect_shell_elements(
                                renderer,
                                &state.shell,
                                &state.shell.dock_layout,
                                (state.pointer_location.x, state.pointer_location.y),

                                &mut top_bar_glass,
                                &mut launcher_glass,
                                &mut dock_glass,

                                &top_bar_shadow_buffer,
                                &top_bar_highlight_buffer,
                                &top_bar_border_buffer,

                                &dock_shadow_buffer,
                                &dock_highlight_buffer,
                                &dock_border_buffer,

                                &shell_text,
                                &tray,

                                scale,
                            );


                        // ------------------------------------------------
                        // SHELL + CLIENT WINDOWS
                        // ------------------------------------------------
                        let output_size =
                            state
                                .space
                                .output_geometry(&output)
                                .map(|geometry| geometry.size)
                                .unwrap_or_else(|| {
                                    mode.size
                                        .to_f64()
                                        .to_logical(scale)
                                        .to_i32_round()
                                });

                        let top_bar_height = state.shell.top_bar.map(|p| p.size.1).unwrap_or(0);

                        let elements = renderer::collect_frame_elements(
                            renderer,
                            &state.space,
                            scale,
                            &wallpaper,
                            output_size,
                            shell_elements,
                            std::iter::empty(),
                            &mut window_chrome,      // (From previous step)
                            &state.popups,           // (From previous step)
                            &state.notifications.active, // <--- ADD THIS
                            top_bar_height,        // <--- ADD THIS
                            &state.auth,
                            &state.auth,
                            &state.osd,             // <-- ADDED
                            state.night_light,      // <-- ADDED
                            state.current_workspace,      // <--- ADD
                            state.workspace_swipe_x,      // <--- ADD
                            output_size.w,                // <--- ADD
                        )?;



                        // ------------------------------------------------
                        // DAMAGE TRACKER
                        // ------------------------------------------------

                        damage_tracker
                            .render_output(
                                renderer,
                                &mut framebuffer,
                                age,
                                &elements,
                                renderer::clear_color(
                                    &state.home_screen,
                                ),
                            )
                            .map_err(|err| match err {
                                DamageTrackerError::Rendering(
                                    err,
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
                        } else {
                            // First real frame on screen -> tell
                            // mitos-init the session is genuinely ready.
                            if !ready_sent {
                                ready_sent = true;
                                notify::send_ready();
                            }
                        }
                    }

                    full_redraw_frames =
                        full_redraw_frames.saturating_sub(1);

                    // ------------------------------------------------
                    // Frame callbacks
                    // ------------------------------------------------

                    let now =
                        state.clock.now();

                    for window in state.space.elements() {
                        window.send_frame(
                            &output,
                            now,
                            Some(Duration::from_secs(1)),
                            |_, _| Some(output.clone()),
                        );
                    }
                }

                // ----------------------------------------------------
                // GPU CONTEXT LOST
                // ----------------------------------------------------

                Err(
                    SwapBuffersError::ContextLost(err)
                ) => {
                    tracing::error!(
                        "MITOS GUI: critical rendering error, exiting: {err}"
                    );

                    break;
                }

                // ----------------------------------------------------
                // OTHER RENDER ERROR
                // ----------------------------------------------------

                Err(err) => {
                    tracing::warn!(
                        "MITOS GUI: render error: {err}"
                    );
                }
            }
        }

        // ========================================================
        // MAINTENANCE
        // ========================================================

        state.space.refresh();
        state.popups.cleanup();

        display.dispatch_clients(
            &mut state,
        )?;

        display.flush_clients()?;

        // ========================================================
        // VSYNC / IDLE CPU FIX
        // ========================================================
        // If we just rendered (damage existed), pump events immediately.
        // If we are idle (no damage), sleep for 16ms to save CPU (~60fps).
        let timeout = if should_render {
            Duration::from_millis(1)
        } else {
            Duration::from_millis(16)
        };

        event_loop.dispatch(
            Some(timeout),
            &mut state,
        )?;
    }

    Ok(())
}
