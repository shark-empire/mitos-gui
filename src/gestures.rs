//! Stage 5: Touchpad Gesture Engine.
//! Handles 1:1 workspace swiping.

use smithay::backend::input::{
    InputBackend,
    GestureBeginEvent,
    GestureEventTrait,
    GestureSwipeUpdateEvent,
};
use crate::state::{MitosGuiState, WORKSPACE_COUNT};

pub fn handle_swipe_begin<B: InputBackend>(
    state: &mut MitosGuiState,
    event: B::GestureSwipeBeginEvent,
) {
    // Only trigger on 3 or 4 finger horizontal swipes
    if event.fingers() >= 3 {
        state.workspace_swipe_x = 0.0;
    }
}

pub fn handle_swipe_update<B: InputBackend>(
    state: &mut MitosGuiState,
    event: B::GestureSwipeUpdateEvent,
) {
    if event.fingers() >= 3 {
        // event.delta_x() is usually in pixels. We normalize it to screen width.
        // Assuming ~800px width for normalization, adjust as needed.
        let delta = event.delta_x() / 800.0;

        // Accumulate the swipe offset
        state.workspace_swipe_x += delta;

        // Clamp so you can't swipe past the first/last workspace on the active monitor
        let active = state.active_output_name();
        let current_ws = state.current_workspace.get(&active).copied().unwrap_or(0);

        let max_offset = current_ws as f64;
        let min_offset = -((WORKSPACE_COUNT - 1 - current_ws) as f64);

        state.workspace_swipe_x = state.workspace_swipe_x.clamp(min_offset, max_offset);
        state.pending_full_redraw = true;
    }
}

pub fn handle_swipe_end<B: InputBackend>(
    state: &mut MitosGuiState,
    _event: B::GestureSwipeEndEvent,
) {
    // Snap to the nearest workspace based on swipe distance
    let threshold = 0.2; // 20% of the screen width

    let active = state.active_output_name();
    let current_ws = state.current_workspace.get(&active).copied().unwrap_or(0);

    if state.workspace_swipe_x > threshold {
        if current_ws > 0 {
            state.switch_workspace(&active, current_ws - 1);
        }
    } else if state.workspace_swipe_x < -threshold {
        if current_ws < WORKSPACE_COUNT - 1 {
            state.switch_workspace(&active, current_ws + 1);
        }
    }

    state.workspace_swipe_x = 0.0;
    state.pending_full_redraw = true;
}
