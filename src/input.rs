//! Routes raw input-backend events to the keyboard/pointer handlers.
//!
//! Winit forwards host keyboard/mouse events as `InputEvent<WinitInput>`
//! -- the same generic shape a real libinput backend produces -- so
//! this dispatcher is written against `InputBackend` generically and
//! doesn't need to change when Stage 5 swaps winit for real hardware.

use smithay::backend::input::{InputBackend, InputEvent};
use smithay::output::Output;

use crate::keyboard::handle_keyboard_key;
use crate::pointer::{handle_pointer_axis, handle_pointer_button, handle_pointer_motion_absolute};
use crate::state::MitosGuiState;
use crate::gestures; 

pub fn process_input_event<B: InputBackend>(state: &mut MitosGuiState, output: &Output, event: InputEvent<B>) {
    match event {
        InputEvent::Keyboard { event } => handle_keyboard_key::<B>(state, event),
        InputEvent::PointerMotionAbsolute { event } => {
            handle_pointer_motion_absolute::<B>(state, output, event)
        }
        InputEvent::PointerButton { event } => handle_pointer_button::<B>(state, event),
        InputEvent::PointerAxis { event } => handle_pointer_axis::<B>(state, event),
        // Relative motion, gestures, touch, tablet, device hotplug --
        // winit's virtual device never produces these, and a real
        // source for them (libinput) doesn't exist until Stage 5.
                // --- STAGE 5: TOUCHPAD GESTURES ---
        InputEvent::GestureSwipeBegin { event } => gestures::handle_swipe_begin::<B>(state, event),
        InputEvent::GestureSwipeUpdate { event } => gestures::handle_swipe_update::<B>(state, event),
        InputEvent::GestureSwipeEnd { event } => gestures::handle_swipe_end::<B>(state, event),
        _ => {}
    }
}
