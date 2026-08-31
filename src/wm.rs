//! Stage 4 window manager.
//!
//! Responsibilities:
//! - focus management
//! - interactive move / resize
//! - maximize / minimize / fullscreen / close
//! - left / right snap layouts
//!
//! Geometry policy lives here. Rendering stays in renderer.rs.

use std::sync::{Mutex, MutexGuard};

use smithay::desktop::Window;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Size, SERIAL_COUNTER};
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::shell::xdg::ToplevelSurface;

use crate::state::MitosGuiState;

// ============================================================================
// WINDOW METADATA
// ============================================================================

/// Per-window WM metadata stored in the window's user data.
#[derive(Clone, Debug, Default)]
pub struct WindowMeta {
    /// Geometry saved while maximized / fullscreen / snapped.
    pub saved: Option<Rectangle<i32, Logical>>,
    pub maximized: bool,
    pub fullscreen: bool,
    pub workspace: usize,
}

impl Default for WindowMeta {
    fn default() -> Self {
        Self {
            saved: None,
            maximized: false,
            fullscreen: false,
            workspace: 0,
        }
    }
}

pub fn meta(window: &Window) -> MutexGuard<'_, WindowMeta> {
    window
        .user_data()
        .insert_if_missing(|| Mutex::new(WindowMeta::default()));

    window
        .user_data()
        .get::<Mutex<WindowMeta>>()
        .expect("window meta just inserted")
        .lock()
        .unwrap()
}

// ============================================================================
// INTERACTIVE MOVE / RESIZE
// ============================================================================

#[derive(Clone, Debug)]
pub enum InteractiveAction {
    Move {
        window: Window,
        offset: Point<f64, Logical>,
    },
    Resize {
        window: Window,
        start_size: Size<i32, Logical>,
        start_pointer: Point<f64, Logical>,
    },
}

impl InteractiveAction {
    pub fn window(&self) -> &Window {
        match self {
            InteractiveAction::Move { window, .. } => window,
            InteractiveAction::Resize { window, .. } => window,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum SnapSide {
    Left,
    Right,
}

// ============================================================================
// HELPERS
// ============================================================================

fn toplevel(window: &Window) -> Option<ToplevelSurface> {
    window
        .underlying_surface()
        .wayland()
        .and_then(|surface| surface.toplevel())
        .cloned()
}

fn output_size(state: &MitosGuiState) -> Size<i32, Logical> {
    state
        .output
        .current_mode()
        .map(|mode| Size::from((mode.size.w, mode.size.h)))
        .unwrap_or_else(|| Size::from((1280, 720)))
}

/// Area not covered by the top bar or the dock.
pub fn usable_area(state: &MitosGuiState) -> Rectangle<i32, Logical> {
    let size = output_size(state);

    let top = state.shell.top_bar.map(|p| p.size.1).unwrap_or(0);
    let bottom = state.shell.dock.map(|p| p.size.1 + 20).unwrap_or(0);

    let height = (size.h - top - bottom).max(1);

    Rectangle::new((0, top).into(), (size.w, height))
}

fn current_geometry(
    state: &MitosGuiState,
    window: &Window,
) -> Rectangle<i32, Logical> {
    let location = state
        .space
        .element_location(window)
        .unwrap_or_default();

    Rectangle::new(location, window.geometry().size)
}

// ============================================================================
// FOCUS
// ============================================================================

/// Set WM + keyboard focus, and update toplevel activated states.
pub fn set_focus(state: &mut MitosGuiState, window: Option<Window>) {
    if state.focused_window == window {
        return;
    }

    for w in state.space.elements() {
        let is_focused = window.as_ref() == Some(w);

        if let Some(tl) = toplevel(w) {
            tl.with_pending_state(|s| {
                if is_focused {
                    s.states.set(xdg_toplevel::State::Activated);
                } else {
                    s.states.unset(xdg_toplevel::State::Activated);
                }
            });
            tl.send_configure();
        }
    }

    state.focused_window = window.clone();

    let surface = window
        .as_ref()
        .and_then(|w| w.wl_surface())
        .map(|s| s.into_owned());

    if let Some(keyboard) = state.seat.get_keyboard() {
        keyboard.set_focus(state, surface, SERIAL_COUNTER.next_serial());
    }
    
    // Update dock running indicators when focus changes
    crate::shell_interaction::update_running_state(state);
}

/// Cycle focus through mapped windows (Super+Tab).
pub fn cycle_focus(state: &mut MitosGuiState) {
    let windows: Vec<Window> = state.space.elements().cloned().collect();

    if windows.is_empty() {
        return;
    }

    let next = state
        .focused_window
        .as_ref()
        .and_then(|f| windows.iter().position(|w| w == f))
        .map(|i| (i + 1) % windows.len())
        .unwrap_or(0);

    set_focus(state, Some(windows[next].clone()));
}

// ============================================================================
// WINDOW OPERATIONS
// ============================================================================

/// Ask the focused client to close (Super+Q).
pub fn close_focused(state: &mut MitosGuiState) {
    let Some(window) = state.focused_window.clone() else {
        return;
    };

    if let Some(tl) = toplevel(&window) {
        tl.send_close();
    }
}

// ============================================================================
// CLIENT REQUESTS (Called from compositor.rs XDG shell handlers)
// ============================================================================

/// Handle a client request to maximize the window.
pub fn request_maximize(state: &mut MitosGuiState, window: &Window) {
    let Some(tl) = toplevel(window) else { return; };
    let area = usable_area(state);
    
    let mut m = meta(window);
    if !m.fullscreen && !m.maximized {
        m.saved = Some(current_geometry(state, window));
    }
    m.maximized = true;
    drop(m);

    tl.with_pending_state(|s| {
        s.states.set(xdg_toplevel::State::Maximized);
        s.size = Some(area.size);
    });
    tl.send_configure();

    state.space.map_element(window.clone(), area.loc, true);
}

/// Handle a client request to unmaximize the window.
pub fn request_unmaximize(state: &mut MitosGuiState, window: &Window) {
    let Some(tl) = toplevel(window) else { return; };
    
    let mut m = meta(window);
    m.maximized = false;
    let saved = m.saved.take();
    drop(m);

    tl.with_pending_state(|s| {
        s.states.unset(xdg_toplevel::State::Maximized);
        s.size = None;
    });
    tl.send_configure();

    if let Some(geo) = saved {
        state.space.map_element(window.clone(), geo.loc, true);
    }
}

/// Handle a client request to fullscreen the window.
pub fn request_fullscreen(state: &mut MitosGuiState, window: &Window) {
    let Some(tl) = toplevel(window) else { return; };
    let size = output_size(state);
    
    let mut m = meta(window);
    if !m.maximized && !m.fullscreen {
        m.saved = Some(current_geometry(state, window));
    }
    m.fullscreen = true;
    drop(m);

    tl.with_pending_state(|s| {
        s.states.set(xdg_toplevel::State::Fullscreen);
        s.size = Some(size);
    });
    tl.send_configure();

    state.space.map_element(window.clone(), (0, 0).into(), true);
}

/// Handle a client request to unfullscreen the window.
pub fn request_unfullscreen(state: &mut MitosGuiState, window: &Window) {
    let Some(tl) = toplevel(window) else { return; };
    
    let mut m = meta(window);
    m.fullscreen = false;
    let saved = m.saved.take();
    drop(m);

    tl.with_pending_state(|s| {
        s.states.unset(xdg_toplevel::State::Fullscreen);
        s.size = None;
    });
    tl.send_configure();

    if let Some(geo) = saved {
        state.space.map_element(window.clone(), geo.loc, true);
    }
}

/// Handle a client request to minimize the window.
pub fn request_minimize(state: &mut MitosGuiState, window: &Window) {
    let mut m = meta(window);
    if !m.maximized && !m.fullscreen && m.saved.is_none() {
        m.saved = Some(current_geometry(state, window));
    }
    drop(m);

    state.space.unmap_elem(window);
    state.minimized.push(window.clone());
    
    // Focus the topmost remaining window
    let next_focus = state.space.elements().next_back().cloned();
    set_focus(state, next_focus);
}

// ============================================================================
// COMPOSITOR SHORTCUTS
// ============================================================================

/// Toggle maximized (Super+Up).
pub fn toggle_maximize(state: &mut MitosGuiState) {
    let Some(window) = state.focused_window.clone() else {
        return;
    };
    
    let is_maximized = meta(&window).maximized;
    if is_maximized {
        request_unmaximize(state, &window);
    } else {
        request_maximize(state, &window);
    }
}

/// Toggle fullscreen (Super+F).
pub fn toggle_fullscreen(state: &mut MitosGuiState) {
    let Some(window) = state.focused_window.clone() else {
        return;
    };
    
    let is_fullscreen = meta(&window).fullscreen;
    if is_fullscreen {
        request_unfullscreen(state, &window);
    } else {
        request_fullscreen(state, &window);
    }
}

/// Minimize the focused window (Super+Down).
pub fn minimize_focused(state: &mut MitosGuiState) {
    let Some(window) = state.focused_window.clone() else {
        return;
    };

    request_minimize(state, &window);
}

/// Restore the most recently minimized window (Super+Shift+Down).
pub fn restore_minimized(state: &mut MitosGuiState) {
    let Some(window) = state.minimized.pop() else {
        return;
    };

    let loc = meta(&window)
        .saved
        .map(|g| g.loc)
        .unwrap_or_else(|| {
            let n = state.space.elements().count() as i32;
            (n * 24, n * 24).into()
        });

    state.space.map_element(window.clone(), loc, true);
    set_focus(state, Some(window));
}

/// Snap to left/right half (Super+Left / Super+Right).
pub fn snap(state: &mut MitosGuiState, side: SnapSide) {
    let Some(window) = state.focused_window.clone() else {
        return;
    };
    let Some(tl) = toplevel(&window) else {
        return;
    };

    let area = usable_area(state);
    let half = (area.size.w / 2).max(1);

    let (x, w) = match side {
        SnapSide::Left => (0, half),
        SnapSide::Right => (area.size.w - half, half),
    };

    {
        let mut m = meta(&window);
        
        // Clear maximized/fullscreen states if snapping
        if m.maximized || m.fullscreen {
            tl.with_pending_state(|s| {
                s.states.unset(xdg_toplevel::State::Maximized);
                s.states.unset(xdg_toplevel::State::Fullscreen);
            });
            m.maximized = false;
            m.fullscreen = false;
        }
        
        if m.saved.is_none() {
            m.saved = Some(current_geometry(state, &window));
        }
    }

    tl.with_pending_state(|s| {
        s.size = Some(Size::from((w, area.size.h)));
    });
    tl.send_configure();

    state
        .space
        .map_element(window, (x, area.loc.y).into(), true);
}

// ============================================================================
// INTERACTIVE MOVE / RESIZE
// ============================================================================

/// Begin moving a window (Super + left drag).
pub fn begin_move(state: &mut MitosGuiState, window: Window) {
    // If the window is maximized, unmaximize it before moving
    if meta(&window).maximized {
        request_unmaximize(state, &window);
    }

    let loc = state
        .space
        .element_location(&window)
        .unwrap_or_default();

    let offset = state.pointer_location - loc.to_f64();

    state.interactive = Some(InteractiveAction::Move { window, offset });
}

/// Begin resizing a window from its bottom-right (Super + right drag).
pub fn begin_resize(state: &mut MitosGuiState, window: Window) {
    // If the window is maximized, unmaximize it before resizing
    if meta(&window).maximized {
        request_unmaximize(state, &window);
    }

    let start_size = window.geometry().size;
    let start_pointer = state.pointer_location;

    state.interactive = Some(InteractiveAction::Resize {
        window,
        start_size,
        start_pointer,
    });
}

/// Apply pointer motion to the active move/resize.
pub fn update_interactive(state: &mut MitosGuiState) {
    let Some(action) = state.interactive.clone() else {
        return;
    };

    match action {
        InteractiveAction::Move { window, offset } => {
            let loc = state.pointer_location - offset;
            state.space.map_element(window, loc.to_i32_round(), true);
        }

        InteractiveAction::Resize {
            window,
            start_size,
            start_pointer,
        } => {
            let delta = state.pointer_location - start_pointer;

            let w = (start_size.w as f64 + delta.x).max(200.0) as i32;
            let h = (start_size.h as f64 + delta.y).max(150.0) as i32;

            if let Some(tl) = toplevel(&window) {
                tl.with_pending_state(|s| {
                    s.size = Some(Size::from((w, h)));
                });
                tl.send_configure();
            }
        }
    }
}

/// End the active move/resize (button release).
pub fn end_interactive(state: &mut MitosGuiState) {
    state.interactive = None;
}

// ============================================================================
// LIFECYCLE CLEANUP
// ============================================================================

/// Remove every reference to a destroyed window.
pub fn cleanup_destroyed(state: &mut MitosGuiState, surface: &WlSurface) {
    if let Some(focused) = &state.focused_window {
        if focused.wl_surface().as_deref() == Some(surface) {
            // Try to focus the next available window instead of dropping to None
            let next_focus = state.space.elements()
                .filter(|w| w.wl_surface().as_deref() != Some(surface))
                .next_back()
                .cloned();
            set_focus(state, next_focus);
        }
    }

    state
        .minimized
        .retain(|w| w.wl_surface().as_deref() != Some(surface));

    if let Some(action) = &state.interactive {
        if action.window().wl_surface().as_deref() == Some(surface) {
            state.interactive = None;
        }
    }
    
    // Update dock indicators since a window was destroyed
    crate::shell_interaction::update_running_state(state);
}
