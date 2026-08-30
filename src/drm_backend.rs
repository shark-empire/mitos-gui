//! Stage 11 production backend — Phase 2.
//!
//! Full hardware path:
//!
//!     libseat session -> /dev/dri/cardN -> DrmDevice -> DrmSurface
//!     -> GBM allocator -> EGL -> GLES renderer -> DrmCompositor
//!     -> vblank-driven frame loop + libinput + READY=1
//!
//! The Winit path in main.rs remains the development backend.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use calloop::EventLoop;

use nix::fcntl::OFlag;
use nix::sys::stat::Mode;

use smithay::backend::allocator::gbm::{
    GbmAllocator,
    GbmBufferFlags,
    GbmDevice,
};
use smithay::backend::allocator::Fourcc;
use smithay::backend::drm::{
    compositor::DrmCompositor,
    DrmDevice,
    DrmDeviceFd,
    DrmEvent,
};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::libinput::{
    LibinputInputBackend,
    LibinputSession,
};
use smithay::backend::renderer::element::solid::SolidColorBuffer;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::Color32F;
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::Session;

use smithay::reexports::drm::control::{
    connector,
    Device as ControlDevice,
};

use smithay::input::keyboard::XkbConfig;
use smithay::input::SeatState;
use smithay::output::{
    Mode,
    Output,
    PhysicalProperties,
    Subpixel,
};
use smithay::utils::{Scale, Transform};
use smithay::reexports::wayland_server::{
    Display,
    ListeningSocket,
};
use smithay::wayland::{
    compositor::CompositorState,
    shell::xdg::XdgShellState,
    shm::ShmState,
};

use crate::state::MitosGuiState;

fn pick_card() -> Option<String> {
    for i in 0..8 {
        let path = format!("/dev/dri/card{i}");
        if Path::new(&path).exists() {
            return Some(path);
        }
    }
    None
}

/// Boot MITOS GUI directly on real hardware.
pub fn run_drm() -> Result<(), Box<dyn std::error::Error>> {
    println!("MITOS GUI: production DRM backend starting");

    // ============================================================
    // EVENT LOOP
    // ============================================================
    let mut event_loop: EventLoop<MitosGuiState> =
        EventLoop::try_new()?;

    // ============================================================
    // LIBSEAT SESSION
    //
    // ADJUST: smithay 0.7 LibSeatSession::new return shape.
    // ============================================================
    let (session, notify) = LibSeatSession::new()?;

    event_loop
        .handle()
        .insert_source(notify, |(), _, _| {})?;

    println!("MITOS GUI: libseat session active");

    // ============================================================
    // WAYLAND DISPLAY
    // ============================================================
    let mut display: Display<MitosGuiState> = Display::new()?;

    let listening_socket =
        ListeningSocket::bind_auto("wayland", 0..10)
            .expect("Failed to create Wayland listening socket");

    println!(
        "MITOS GUI: Wayland socket created at {:?}",
        listening_socket.socket_name()
    );

    let display_handle = display.handle();

    // ============================================================
    // DRM DEVICE
    // ============================================================
    let node = pick_card().ok_or("no /dev/dri/cardN found")?;

    let fd = session.open(
        Path::new(&node),
        OFlag::RDWR | OFlag::CLOEXEC,
        Mode::empty(),
    )?;

    let fd = DrmDeviceFd::new(fd);

    let (mut drm, drm_event_source) =
        DrmDevice::new(fd.clone(), false)?;

    println!("MITOS GUI: DRM device opened on {node}");

    // ============================================================
    // CONNECTOR / CRTC / MODE
    // ============================================================
    let res = drm.resource_handles()?;

    let conn_handle = *res
        .connectors()
        .iter()
        .find(|c| {
            drm.connector_info(c)
                .map(|i| i.state() == connector::State::Connected)
                .unwrap_or(false)
        })
        .ok_or("no connected DRM outputs")?;

    let conn_info = drm.connector_info(&conn_handle)?;

    let mode = *conn_info
        .modes()
        .first()
        .ok_or("connected output has no modes")?;

    let crtc = conn_info
        .current_crtc()
        .or_else(|| {
            conn_info
                .encoders()
                .iter()
                .flat_map(|e| drm.encoder_info(e).ok())
                .find_map(|e| e.crtc())
        })
        .or_else(|| res.crtcs().first().copied())
        .ok_or("no usable CRTC")?;

    let size = (mode.size().0 as i32, mode.size().1 as i32);
    let refresh = mode.vrefresh() as i32 * 1000;

    println!(
        "MITOS GUI: mode {}x{}@{}mHz on {:?}",
        size.0,
        size.1,
        refresh,
        conn_info.interface()
    );

    let surface =
        drm.create_surface(crtc, mode, &[conn_handle])?;

    // ============================================================
    // OUTPUT
    // ============================================================
    let output = Output::new(
        format!("MITOS-DRM-0"),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "MITOS".into(),
            model: "DRM".into(),
        },
    );

    let _output_global =
        output.create_global::<MitosGuiState>(&display_handle);

    let out_mode = Mode {
        size: size.into(),
        refresh,
    };

    output.change_current_state(
        Some(out_mode),
        Some(Transform::Normal),
        Some(Scale::Integer(1)),
        Some((0, 0).into()),
    );
    output.set_preferred(out_mode);

    // ============================================================
    // GBM / EGL / GLES
    // ============================================================
    let gbm = GbmDevice::new(fd.clone())?;

    let egl_display = EGLDisplay::new(gbm.clone())?; // ADJUST if 0.7 differs
    let egl_context = EGLContext::new(&egl_display, None)?;
    let renderer = unsafe { GlesRenderer::new(egl_context)? };

    println!("MITOS GUI: GLES renderer on GBM/EGL (production)");

    // ============================================================
    // DRM COMPOSITOR
    //
    // ADJUST: smithay 0.7 DrmCompositor::new argument list.
    // ============================================================
    let compositor = DrmCompositor::new(
        &output,
        surface,
        None,
        GbmAllocator::new(
            fd.clone(),
            GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
        ),
        gbm.clone(),
        renderer.dmabuf_formats(),
        None,
        fd.clone(),
        None,
    )?;

    // Renderer + compositor shared with calloop closures.
    let runtime =
        Rc::new(RefCell::new((compositor, renderer)));

    // ============================================================
    // VBLANK HANDLING
    // ============================================================
    {
        let runtime = runtime.clone();

        event_loop.handle().insert_source(
            drm_event_source,
            move |event, _, state: &mut MitosGuiState| match event {
                DrmEvent::VBlank(_) => {
                    if let Ok(mut rt) = runtime.try_borrow_mut() {
                        let _ = rt.0.submit_frame();
                    }
                    state.drm_vblank = true;
                }
                DrmEvent::Error(err) => {
                    tracing::error!(
                        "MITOS GUI: DRM event error: {err:?}"
                    );
                }
            },
        )?;
    }

    // ============================================================
    // LIBINPUT (real keyboard / mouse / touchpad)
    //
    // ADJUST: smithay 0.7 LibinputSession::new shape.
    // ============================================================
    let libinput_session = LibinputSession::new(session.clone());
    let input_backend = LibinputInputBackend::new(libinput_session);

    {
        let output = output.clone();

        event_loop.handle().insert_source(
            input_backend,
            move |event, _, state: &mut MitosGuiState| {
                crate::input::process_input_event(
                    state, &output, event,
                );
            },
        )?;
    }

    println!("MITOS GUI: libinput attached (seat0)");

    // ============================================================
    // WAYLAND PROTOCOL STATE
    // ============================================================
    let compositor_state =
        CompositorState::new::<MitosGuiState>(&display_handle);

    let mut shm_state = ShmState::new::<MitosGuiState>(
        &display_handle,
        vec![],
    );

    shm_state.update_formats(
        runtime.borrow().1.shm_formats(),
    );

    let xdg_shell_state =
        XdgShellState::new::<MitosGuiState>(&display_handle);

    // ============================================================
    // SEAT
    // ============================================================
    let mut seat_state = SeatState::<MitosGuiState>::new();

    let mut seat =
        seat_state.new_wl_seat(&display_handle, "seat0");

    seat.add_keyboard(XkbConfig::default(), 200, 25)?;
    seat.add_pointer();

    // ============================================================
    // MITOS STATE + SHELL
    // ============================================================
    let home_screen = crate::desktop::HomeScreenConfig::load();
    crate::theme::MitosTheme::apply_runtime(&home_screen);

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

    let mut wallpaper =
        crate::renderer::Wallpaper::load_default()
            .map_err(|e| format!("MITOS GUI: {e}"))?;

    let mut shell_text = crate::renderer::ShellTextState::new();

    // Glass panels + support buffers (same as Winit path).
    let mut top_bar_glass =
        crate::renderer::create_glass_panel_element(
            &mut runtime.borrow_mut().1,
        )?;
    let mut launcher_glass =
        crate::renderer::create_glass_panel_element(
            &mut runtime.borrow_mut().1,
        )?;
    let mut dock_glass =
        crate::renderer::create_glass_panel_element(
            &mut runtime.borrow_mut().1,
        )?;

    let mut top_bar_shadow =
        SolidColorBuffer::new((0, 0), crate::renderer::shadow_color());
    let mut top_bar_highlight = SolidColorBuffer::new(
        (0, 0),
        crate::renderer::glass_highlight_color(),
    );
    let mut top_bar_border = SolidColorBuffer::new(
        (0, 0),
        Color32F::new(0.0, 0.0, 0.0, 0.0),
    );

    let mut dock_shadow =
        SolidColorBuffer::new((0, 0), crate::renderer::shadow_color());
    let mut dock_highlight = SolidColorBuffer::new(
        (0, 0),
        crate::renderer::glass_highlight_color(),
    );
    let mut dock_border = SolidColorBuffer::new(
        (0, 0),
        Color32F::new(0.0, 0.0, 0.0, 0.0),
    );

    let mut full_redraw_frames: u8 = 4;
    let mut ready_sent = false;

    println!("MITOS GUI: DRM compositor ready");
    println!("MITOS GUI: event loop running (production)");

    // ============================================================
    // MAIN LOOP (vblank-driven)
    // ============================================================
    loop {
        let needs_render =
            state.drm_vblank || state.pending_full_redraw || full_redraw_frames > 0;

        let timeout = if needs_render {
            Duration::from_millis(1)
        } else {
            Duration::from_millis(16)
        };

        event_loop.dispatch(Some(timeout), &mut state)?;

        // --------------------------------------------------------
        // Wayland clients
        // --------------------------------------------------------
        if let Some(stream) = listening_socket.accept()? {
            display.handle().insert_client(
                stream,
                Arc::new(
                    crate::compositor::MitosClientState::default(),
                ),
            )?;
        }

        // --------------------------------------------------------
        // Live config reload
        // --------------------------------------------------------
        if state.pending_full_redraw {
            state.pending_full_redraw = false;
            full_redraw_frames = 4;
        }

        if shell_text.refresh(&state.shell) {
            full_redraw_frames = full_redraw_frames.max(1);
        }

        // --------------------------------------------------------
        // RENDER + PAGE FLIP
        // --------------------------------------------------------
        if state.drm_vblank || full_redraw_frames > 0 {
            state.drm_vblank = false;
            full_redraw_frames = full_redraw_frames.saturating_sub(1);

            let mut rt = runtime.borrow_mut();
            let (compositor, renderer) = &mut *rt;

            let scale = Scale::from(
                output.current_scale().fractional_scale(),
            );

            let shell_elements =
                crate::renderer::collect_shell_elements(
                    renderer,
                    &state.shell,
                    &state.shell.dock_layout,
                    (state.pointer_location.x, state.pointer_location.y),
                    &mut top_bar_glass,
                    &mut launcher_glass,
                    &mut dock_glass,
                    &top_bar_shadow,
                    &top_bar_highlight,
                    &top_bar_border,
                    &dock_shadow,
                    &dock_highlight,
                    &dock_border,
                    &shell_text,
                    scale,
                );

            let output_size = size.into();

            let elements = match crate::renderer::collect_frame_elements(
                renderer,
                &state.space,
                scale,
                &wallpaper,
                output_size,
                shell_elements,
                std::iter::empty(),
            ) {
                Ok(e) => e,
                Err(err) => {
                    tracing::warn!("MITOS GUI: frame build error: {err}");
                    continue;
                }
            };

            // ADJUST: render_frame / queue_frame signatures.
            match compositor.render_frame(
                renderer,
                &elements,
                Color32F::new(0.0, 0.0, 0.0, 1.0),
            ) {
                Ok(frame) => {
                    if frame.damage.is_some() {
                        if compositor.queue_frame().is_ok() {
                            if !ready_sent {
                                ready_sent = true;
                                crate::notify::send_ready();
                            }
                        }
                    } else {
                        let _ = compositor.reset_frame();
                    }

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
                Err(err) => {
                    tracing::warn!("MITOS GUI: render error: {err}");
                }
            }
        }

        // --------------------------------------------------------
        // Maintenance
        // --------------------------------------------------------
        state.space.refresh();
        state.popups.cleanup();
        display.dispatch_clients(&mut state)?;
        display.flush_clients()?;
    }
}
