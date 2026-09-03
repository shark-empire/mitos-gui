//! Stage 11 production backend — Phase 3 (Multi-Monitor & Hotplug).
//!
//! Full hardware path:
//!
//!     libseat session -> /dev/dri/cardN -> DrmDevice -> DrmSurface
//!     -> GBM allocator -> EGL -> GLES renderer -> DrmCompositor
//!     -> vblank-driven frame loop + libinput + READY=1
//!
//! The Winit path in main.rs remains the development backend.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use calloop::EventLoop;
use calloop::timer::{Timer, TimeoutAction};

use nix::fcntl::OFlag;

use smithay::backend::allocator::gbm::{
    GbmAllocator,
    GbmBufferFlags,
    GbmDevice,
};
use smithay::backend::allocator::Fourcc;
use smithay::backend::drm::{
    compositor::{DrmCompositor, FrameFlags},
    DrmDevice,
    DrmDeviceFd,
    DrmEvent,
    exporter::gbm::GbmFramebufferExporter,
};
use smithay::backend::egl::{EGLContext, EGLDisplay};

use smithay::backend::renderer::element::solid::SolidColorBuffer;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::{Color32F, ImportDma, ImportMemWl};
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::Session;
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};

use smithay::reexports::drm::control::{
    connector,
    Device as ControlDevice,
};

use smithay::reexports::input::Libinput;

use smithay::input::keyboard::XkbConfig;
use smithay::input::SeatState;
use smithay::output::{
    Mode as SmithayMode,
    Output,
    PhysicalProperties,
    Scale as OutputScale,
    Subpixel,
};

use smithay::utils::{Buffer, Scale, Size, Transform};
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

struct DrmOutputState {
    output: Output,
    compositor: DrmCompositor<
        GbmAllocator<DrmDeviceFd>,
        GbmFramebufferExporter<DrmDeviceFd>, // <-- FIXED generic type
        (),
        DrmDeviceFd,
    >,
}

fn create_output(
    drm: &mut DrmDevice,
    renderer: &mut GlesRenderer,
    _fd: &DrmDeviceFd,
    gbm: &GbmDevice<DrmDeviceFd>,
    display_handle: &smithay::reexports::wayland_server::DisplayHandle,
    conn_handle: connector::Handle,
) -> Result<DrmOutputState, Box<dyn std::error::Error>> {
    let conn_info = drm.get_connector(conn_handle, false)?;
    if conn_info.state() != connector::State::Connected {
        return Err("Not connected".into());
    }

    let mode = *conn_info.modes().first().ok_or("no modes")?;
    let crtc = conn_info.current_crtc()
        .or_else(|| conn_info.encoders().iter().flat_map(|e| drm.get_encoder(*e).ok()).find_map(|e| e.crtc()))
        .or_else(|| drm.resource_handles().ok()?.crtcs().first().copied())
        .ok_or("no crtc")?;

    let surface = drm.create_surface(crtc, mode, &[conn_handle])?;

    let output = Output::new(
        format!("MITOS-DRM-{}", conn_handle.into()),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "MITOS".into(),
            model: "DRM".into(),
        },
    );
    output.create_global::<MitosGuiState>(display_handle);

    let size = (mode.size().0 as i32, mode.size().1 as i32);
    let out_mode = SmithayMode { size: size.into(), refresh: mode.vrefresh() as i32 * 1000 }; // <-- ALIASED
    
    output.change_current_state(
        Some(out_mode), 
        Some(Transform::Normal), 
        Some(OutputScale::Integer(1)), 
        Some((0, 0).into())
    );
    output.set_preferred(out_mode);

    let color_formats = [Fourcc::Argb8888, Fourcc::Xrgb8888];
    let renderer_formats: Vec<_> = renderer.dmabuf_formats().collect();
    let cursor_size = Size::<u32, Buffer>::from((64, 64));

    let compositor = DrmCompositor::new(
        &output,
        surface,
        None,
        GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT),
        GbmFramebufferExporter::new(gbm.clone(), None), // <-- CORRECT exporter
        color_formats,
        renderer_formats,
        cursor_size,
        Some(gbm.clone()),
    )?;

    Ok(DrmOutputState { output, compositor })
}

pub fn run_drm() -> Result<(), Box<dyn std::error::Error>> {
    println!("MITOS GUI: production DRM backend starting (Multi-Monitor)");

    let mut event_loop: EventLoop<MitosGuiState> = EventLoop::try_new()?;
    let (session, notify) = LibSeatSession::new()?;
    event_loop.handle().insert_source(notify, |_event, _, _| {})?;
    println!("MITOS GUI: libseat session active");

    let mut display: Display<MitosGuiState> = Display::new()?;
    let listening_socket = ListeningSocket::bind_auto("wayland", 0..10)?;
    println!("MITOS GUI: Wayland socket created at {:?}", listening_socket.socket_name());
    let display_handle = display.handle();

    let node = pick_card().ok_or("no /dev/dri/cardN found")?;
    let fd = session.open(Path::new(&node), OFlag::O_RDWR | OFlag::O_CLOEXEC)?;
    let fd = DrmDeviceFd::new(fd.into());

    let (drm, drm_event_source) = DrmDevice::new(fd.clone(), false)?;
    println!("MITOS GUI: DRM device opened on {node}");

    let drm_rc = Rc::new(RefCell::new(drm));
    
    let gbm = GbmDevice::new(fd.clone())?;
    let egl_display = EGLDisplay::new(gbm.clone())?;
    let egl_context = EGLContext::new(&egl_display)?;
    let renderer = unsafe { GlesRenderer::new(egl_context)? };
    println!("MITOS GUI: GLES renderer on GBM/EGL (production)");
    
    let renderer_rc = Rc::new(RefCell::new(renderer));
    let outputs_rc = Rc::new(RefCell::new(HashMap::<connector::Handle, DrmOutputState>::new()));

    // Initial output setup
    {
        let mut drm = drm_rc.borrow_mut();
        let mut renderer = renderer_rc.borrow_mut();
        let res = drm.resource_handles()?;
        for conn_handle in res.connectors() {
            if let Ok(state) = create_output(&mut drm, &mut renderer, &fd, &gbm, &display_handle, *conn_handle) {
                outputs_rc.borrow_mut().insert(*conn_handle, state);
            }
        }
    }

    // VBlank handler
    {
        let outputs_rc = outputs_rc.clone();
        event_loop.handle().insert_source(
            drm_event_source,
            move |event, _, state: &mut MitosGuiState| match event {
                DrmEvent::VBlank(_crtc) => {
                    state.drm_vblank = true;
                    // Smithay requires frame_submitted() to be called after
                    // each vblank, or the compositor's swapchain will
                    // eventually run out of buffers.
                    if let Ok(mut outputs) = outputs_rc.try_borrow_mut() {
                        for drm_output in outputs.values_mut() {
                            let _ = drm_output.compositor.frame_submitted();
                        }
                    }
                }
                DrmEvent::Error(err) => {
                    tracing::error!("MITOS GUI: DRM event error: {err:?}");
                }
            },
        )?;
    }

    // Hotplug Timer (Polls DRM every 2 seconds for cable changes)
    {
        let drm_rc = drm_rc.clone();
        let renderer_rc = renderer_rc.clone();
        let outputs_rc = outputs_rc.clone();
        let fd = fd.clone();
        let gbm = gbm.clone();
        let display_handle = display_handle.clone();

        let hotplug_timer = Timer::from_duration(Duration::from_secs(2));
        event_loop.handle().insert_source(hotplug_timer, move |_, _, state: &mut MitosGuiState| {
            let mut drm = drm_rc.borrow_mut();
            let mut renderer = renderer_rc.borrow_mut();
            let mut outputs = outputs_rc.borrow_mut();

            if let Ok(res) = drm.resource_handles() {
                let current_conns: Vec<connector::Handle> = res.connectors().to_vec();
                let mut to_add = Vec::new();
                let mut to_remove = Vec::new();

                for conn in &current_conns {
                    if let Ok(info) = drm.get_connector(*conn, false) {
                        let is_connected = info.state() == connector::State::Connected;
                        let is_tracked = outputs.contains_key(conn);

                        if is_connected && !is_tracked {
                            to_add.push(*conn);
                        } else if !is_connected && is_tracked {
                            to_remove.push(*conn);
                        }
                    }
                }

                for conn in to_remove {
                    if let Some(removed) = outputs.remove(&conn) {
                        state.remove_output(&removed.output);
                        println!("MITOS GUI: Hotplug - Removed output {:?}", conn);
                    }
                }

                for conn in to_add {
                    if let Ok(out_state) = create_output(&mut drm, &mut renderer, &fd, &gbm, &display_handle, conn) {
                        state.add_output(out_state.output.clone());
                        outputs.insert(conn, out_state);
                        println!("MITOS GUI: Hotplug - Added output {:?}", conn);
                        state.pending_full_redraw = true;
                    }
                }
            }

            TimeoutAction::ToDuration(Duration::from_secs(2))
        })?;
    }

    // Libinput initialization for Smithay 0.7
    let mut context = Libinput::new_with_udev(LibinputSessionInterface::from(session.clone()));
    context.udev_assign_seat("seat0").unwrap();
    let input_backend = LibinputInputBackend::new(context);

    {
        let outputs_rc = outputs_rc.clone();
        event_loop.handle().insert_source(
            input_backend,
            move |event, _, state: &mut MitosGuiState| {
                let output = outputs_rc.borrow().values().next().map(|s| s.output.clone());
                if let Some(out) = output {
                    crate::input::process_input_event(state, &out, event);
                }
            },
        )?;
    }
    println!("MITOS GUI: libinput attached (seat0)");

    // Wayland state
    let compositor_state = CompositorState::new::<MitosGuiState>(&display_handle);
    let mut shm_state = ShmState::new::<MitosGuiState>(&display_handle, vec![]);
    shm_state.update_formats(renderer_rc.borrow().shm_formats());
    let xdg_shell_state = XdgShellState::new::<MitosGuiState>(&display_handle);

    let mut seat_state = SeatState::<MitosGuiState>::new();
    let mut seat = seat_state.new_wl_seat(&display_handle, "seat0");
    seat.add_keyboard(XkbConfig::default(), 200, 25)?;
    seat.add_pointer();

    let dbus_service = crate::dbus::DbusService::new().expect("Failed to start D-Bus notification service");

    let home_screen = crate::desktop::HomeScreenConfig::load();
    crate::theme::MitosTheme::apply_runtime(&home_screen);

    let initial_outputs: Vec<Output> = outputs_rc.borrow().values().map(|s| s.output.clone()).collect();
    let mut state = MitosGuiState::new(
        compositor_state,
        xdg_shell_state,
        shm_state,
        seat_state,
        seat,
        initial_outputs,
        home_screen,
        dbus_service,
        None,
    );

    let mut wallpaper = crate::renderer::Wallpaper::load_default().map_err(|e| format!("MITOS GUI: {e}"))?;
    let mut shell_text = crate::renderer::ShellTextState::new();
    let mut window_chrome = crate::renderer::WindowChrome::new();
    let mut tray = crate::renderer::TrayState::new();

    let mut top_bar_glass = crate::renderer::create_glass_panel_element(&mut renderer_rc.borrow_mut())?;
    let mut launcher_glass = crate::renderer::create_glass_panel_element(&mut renderer_rc.borrow_mut())?;
    let mut dock_glass = crate::renderer::create_glass_panel_element(&mut renderer_rc.borrow_mut())?;

    let mut top_bar_shadow = SolidColorBuffer::new((0, 0), crate::renderer::shadow_color());
    let mut top_bar_highlight = SolidColorBuffer::new((0, 0), crate::renderer::glass_highlight_color());
    let mut top_bar_border = SolidColorBuffer::new((0, 0), Color32F::new(0.0, 0.0, 0.0, 0.0));
    let mut dock_shadow = SolidColorBuffer::new((0, 0), crate::renderer::shadow_color());
    let mut dock_highlight = SolidColorBuffer::new((0, 0), crate::renderer::glass_highlight_color());
    let mut dock_border = SolidColorBuffer::new((0, 0), Color32F::new(0.0, 0.0, 0.0, 0.0));

    let mut full_redraw_frames: u8 = 4;
    let mut ready_sent = false;

    println!("MITOS GUI: DRM compositor ready (Multi-Monitor)");
    println!("MITOS GUI: event loop running (production)");

    loop {
        let needs_render = state.drm_vblank || state.pending_full_redraw || full_redraw_frames > 0;
        let timeout = if needs_render { Duration::from_millis(1) } else { Duration::from_millis(16) };
        event_loop.dispatch(Some(timeout), &mut state)?;

        if let Some(stream) = listening_socket.accept()? {
            display.handle().insert_client(stream, Arc::new(crate::compositor::MitosClientState::default()))?;
        }

        if state.pending_full_redraw {
            state.pending_full_redraw = false;
            full_redraw_frames = 4;
        }

        if shell_text.refresh(&state.shell) { full_redraw_frames = full_redraw_frames.max(1); }
        if tray.refresh(&state.network, &state.battery, state.volume, state.muted) { state.pending_full_redraw = true; }
        if state.notifications.tick() { state.pending_full_redraw = true; }
        state.poll_dbus();

        if state.drm_vblank || full_redraw_frames > 0 {
            state.drm_vblank = false;
            full_redraw_frames = full_redraw_frames.saturating_sub(1);

            let mut rt = renderer_rc.borrow_mut();
            let renderer = &mut *rt;
            let mut outputs_map = outputs_rc.borrow_mut();

            for (_, drm_output) in outputs_map.iter_mut() {
                let output = &drm_output.output;
                let compositor = &mut drm_output.compositor;

                let output_name = output.name();
                let scale = Scale::from(output.current_scale().fractional_scale());
                let current_ws = state.current_workspace.get(&output_name).copied().unwrap_or(0);
                
                let shell_elements = crate::renderer::collect_shell_elements(
                    renderer, &state.shell, &state.shell.dock_layout,
                    (state.pointer_location.x, state.pointer_location.y),
                    &mut top_bar_glass, &mut launcher_glass, &mut dock_glass,
                    &top_bar_shadow, &top_bar_highlight, &top_bar_border,
                    &dock_shadow, &dock_highlight, &dock_border,
                    &shell_text, &tray, current_ws, crate::state::WORKSPACE_COUNT, scale,
                );

                // Logical (compositor-space) output size, mirroring the winit
                // dev backend: prefer the Space's tracked geometry, falling
                // back to converting the DRM mode's physical size.
                let output_size = state.space.output_geometry(output)
                    .map(|geometry| geometry.size)
                    .unwrap_or_else(|| {
                        let mode_size = output.current_mode().map(|m| m.size).unwrap_or((800, 600).into());
                        mode_size.to_f64().to_logical(scale).to_i32_round()
                    });
                let top_bar_height = state.shell.top_bar.map(|p| p.size.1).unwrap_or(0);

                let elements = match crate::renderer::collect_frame_elements(
                    renderer, &state.space, scale, &wallpaper, output_size,
                    &mut window_chrome, &state.popups, shell_elements, std::iter::empty(),
                    &state.notifications.active, top_bar_height, &state.auth,
                    current_ws, &output_name, state.workspace_swipe_x, output_size.w,
                    &state.osd, state.night_light,
                ) {
                    Ok(e) => e,
                    Err(err) => { tracing::warn!("MITOS GUI: frame build error: {err}"); continue; }
                };

                if state.pending_screenshot {
                    state.pending_screenshot = false;
                    let physical_size = output_size.to_f64().to_physical(scale).to_i32_round();
                    let _ = crate::screenshot::take_screenshot(renderer, physical_size, &elements);
                }

                match compositor.render_frame(renderer, &elements, Color32F::new(0.0, 0.0, 0.0, 1.0), FrameFlags::DEFAULT) {
                    Ok(frame) => {
                        if !frame.is_empty {
                            if compositor.queue_frame(()).is_ok() && !ready_sent {
                                ready_sent = true;
                                crate::notify::send_ready();
                            }
                        }
                        // Empty frame: nothing changed, nothing to queue.
                    }
                    Err(err) => tracing::warn!("MITOS GUI: render error: {err}"),
                }
            }

            let now = state.clock.now();
            for window in state.space.elements() {
                if let Some(output) = state.outputs.first() {
                    window.send_frame(output, now, Some(Duration::from_secs(1)), |_, _| Some(output.clone()));
                }
            }
        }

        state.space.refresh();
        state.popups.cleanup();
        display.dispatch_clients(&mut state)?;
        display.flush_clients()?;
    }
}
