//! Stage 5: Touchpad Gesture Engine.
//! Handles 1:1 workspace swiping.

use smithay::backend::input::{
    InputBackend, SwipeGestureBeginEvent, SwipeGestureUpdateEvent, SwipeGestureEndEvent,
};
use crate::state::MitosGuiState;

pub fn handle_swipe_begin<B: InputBackend>(
    state: &mut MitosGuiState,
    event: B::SwipeGestureBeginEvent,
) {
    // Only trigger on 3 or 4 finger horizontal swipes
    if event.fingers() >= 3 {
        state.workspace_swipe_x = 0.0;
    }
}

pub fn handle_swipe_update<B: InputBackend>(
    state: &mut MitosGuiState,
    event: B::SwipeGestureUpdateEvent,
) {
    if event.fingers() >= 3 {
        // event.delta_x() is usually in pixels. We normalize it to screen width.
        // Assuming ~1000px width for normalization, adjust as needed.
        let delta = event.delta_x() / 800.0; 
        
        // Accumulate the swipe offset
        state.workspace_swipe_x += delta;
        
        // Clamp so you can't swipe past the first/last workspace
        let max_offset = state.current_workspace as f64;
        let min_offset = -((state.workspace_count - 1 - state.current_workspace) as f64);
        
        state.workspace_swipe_x = state.workspace_swipe_x.clamp(min_offset, max_offset);
        state.pending_full_redraw = true;
    }
}

pub fn handle_swipe_end<B: InputBackend>(
    state: &mut MitosGuiState,
    _event: B::SwipeGestureEndEvent,
) {
    // Snap to the nearest workspace based on swipe distance
    let threshold = 0.2; // 20% of the screen width
    
    if state.workspace_swipe_x > threshold {
        if state.current_workspace > 0 {
            state.switch_workspace(state.current_workspace - 1);
        }
    } else if state.workspace_swipe_x < -threshold {
        if state.current_workspace < state.workspace_count - 1 {
            state.switch_workspace(state.current_workspace + 1);
        }
    }
    
    state.workspace_swipe_x = 0.0;
    state.pending_full_redraw = true;
}
