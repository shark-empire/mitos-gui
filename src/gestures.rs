//! Stage 5: Touchpad Gesture Engine.
//! Handles 1:1 workspace swiping.

use smithay::backend::input::{
    GestureBeginEvent,
    GestureEndEvent,
    GestureUpdateEvent,
    InputBackend,
};

use crate::state::{MitosGuiState, WORKSPACE_COUNT};

/// Minimum number of fingers required for workspace switching.
const MIN_FINGERS: u32 = 3;

/// Approximate screen width used to normalize gesture movement.
///
/// This should eventually come from the active output's actual size.
const GESTURE_WIDTH: f64 = 800.0;

/// Fraction of a workspace that must be crossed before committing
/// the workspace change.
const SWIPE_THRESHOLD: f64 = 0.20;

/// Handle the beginning of a touchpad swipe.
///
/// The finger count is available on the begin event, so we determine
/// here whether this gesture is eligible for workspace switching.
pub fn handle_swipe_begin<B: InputBackend>(
    state: &mut MitosGuiState,
    event: B::GestureSwipeBeginEvent,
) {
    if event.fingers() >= MIN_FINGERS {
        state.workspace_swipe_x = 0.0;
        state.pending_full_redraw = true;
    }
}

/// Handle movement during a touchpad swipe.
///
/// `GestureSwipeUpdateEvent` provides movement deltas, but does not
/// provide the finger count. The gesture was already validated during
/// `handle_swipe_begin`.
pub fn handle_swipe_update<B: InputBackend>(
    state: &mut MitosGuiState,
    event: B::GestureSwipeUpdateEvent,
) {
    // event.delta_x() is the horizontal movement reported by libinput.
    //
    // Normalize it into approximately one workspace-width.
    let delta = event.delta_x() / GESTURE_WIDTH;

    // Accumulate the swipe offset.
    state.workspace_swipe_x += delta;

    // Determine the active monitor and workspace.
    let active = state.active_output_name();

    let current_ws = state
        .current_workspace
        .get(&active)
        .copied()
        .unwrap_or(0);

    // Prevent swiping beyond the available workspaces.
    let max_offset = current_ws as f64;
    let min_offset = -((WORKSPACE_COUNT - 1 - current_ws) as f64);

    state.workspace_swipe_x = state
        .workspace_swipe_x
        .clamp(min_offset, max_offset);

    state.pending_full_redraw = true;
}

/// Handle the end of a touchpad swipe.
///
/// The gesture is committed when the user crosses the configured
/// threshold. Otherwise the workspace returns to its original state.
pub fn handle_swipe_end<B: InputBackend>(
    state: &mut MitosGuiState,
    _event: B::GestureSwipeEndEvent,
) {
    let active = state.active_output_name();

    let current_ws = state
        .current_workspace
        .get(&active)
        .copied()
        .unwrap_or(0);

    if state.workspace_swipe_x > SWIPE_THRESHOLD {
        // Swipe toward the previous workspace.
        if current_ws > 0 {
            state.switch_workspace(&active, current_ws - 1);
        }
    } else if state.workspace_swipe_x < -SWIPE_THRESHOLD {
        // Swipe toward the next workspace.
        if current_ws < WORKSPACE_COUNT - 1 {
            state.switch_workspace(&active, current_ws + 1);
        }
    }

    // Always return the gesture offset to zero after the gesture ends.
    state.workspace_swipe_x = 0.0;
    state.pending_full_redraw = true;
}
