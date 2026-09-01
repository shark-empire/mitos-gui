//! Stage 4 window manager.
//!
//! Responsibilities:
//! - focus managementw, area.size
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
#[derive(Clone, Debug,.h)));
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
        request_unmaximize(state, &window); Default)]
pub struct WindowMeta {
    /// Geometry saved while maximized / fullscreen / snapped.
    pub saved: Option<Rectangle<i32, Logical>>,
    pub maximized: bool,
    pub fullscreen: bool,
    pub workspace: HashMap<Output, usize>,
}

impl Default for WindowMeta {
    fn default() -> Self {
        Self {
            saved: None,
            maximized: false,
            fullscreen: false
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
            // Because Smithay's Space uses global coordinates, dragging across 
            // monitor boundaries naturally shifts the window to the next output.
            let loc = state.pointer_location - offset;
            state.space.map_element(window,
            workspace: 0,
        }
    }
}

pub(crate) fn meta(window: &Window) -> MutexGuard<'_, WindowMeta> {
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

/// Get the logical size of the primary output (fallback for global calculations).
pub fn output_size(state: &MitosGuiState) -> Size<i32, Logical> {
    state
        .outputs
        .first()
        .and_then(|o| o.current_mode())
        .map(|mode| Size::from((mode.size.w, mode.size.h)))
        .unwrap_or_else(|| Size::from((1280, 720)))
}

/// Get the logical geometry (position and size) of the output containing the given window.
fn output_geometry_for_window(state: &MitosGuiState, window: &Window) -> Rectangle<i32, Logical> {
    // Check which output the window is currently mapped to
    if let Some(output) = state.space.outputs_for_element(window).next() {
        if let Some(geom) = state.space.output_geometry(output) {
            return geom;
        }
    }
   , loc.to_i32_round(), true);
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
 // Fallback topub fn cleanup_destroyed(state: &mut MitosGuiState, surface: &WlSurface) {
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
        . primary output if window isn't mapped to an output yet
    if let Someminimized
        .retain(|w| w.wl_surface().as_deref() != Some(surface));

    if let Some(action) = &state.interactive {
        if action.window().wl_surface().as_deref() == Some(surface) {
            state.interactive = None;
        }
    }
    
    // Update dock indicators since a window was destroyed
    crate::shell_interaction::update_running_state(state);
}
