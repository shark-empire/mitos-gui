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
    state.report_activity();

    match event {
        InputEvent::Keyboard { event } => handle_keyboard_key::<B>(state, event),

        // The lock/elevation prompt is the only thing allowed to react
        // to input while it's up -- mirrors the "SECURE AUTHENTICATION
        // PROMPT (Highest Priority)" gate at the top of
        // `handle_keyboard_key`, which is the keyboard half of this
        // same rule. Without this, a click still reached the dock
        // (`launch_app`), Super+drag still moved windows, and clicks/
        // scroll/gestures still reached whatever client was focused
        // underneath -- the lock screen only ever gated the keyboard.
        // Pointer position freezing along with everything else (no
        // `handle_pointer_motion_absolute` at all) is deliberate: a
        // static cursor is a normal, unremarkable lock-screen look,
        // and one central gate here is easier to know is airtight than
        // threading an `auth.active` check through every handler this
        // arm would otherwise call.
        _ if state.auth.active => {}

        InputEvent::PointerMotionAbsolute { event } => {
            handle_pointer_motion_absolute::<B>(state, output, event)
        }
        InputEvent::PointerButton { event } => handle_pointer_button::<B>(state, event),
        InputEvent::PointerAxis { event } => handle_pointer_axis::<B>(state, event),
        // Relative motion, touch, tablet, device hotplug --
        // winit's virtual device never produces these, and a real
        // source for them (libinput) doesn't exist until Stage 5.
                // --- STAGE 5: TOUCHPAD GESTURES ---
        InputEvent::GestureSwipeBegin { event } => gestures::handle_swipe_begin::<B>(state, event),
        InputEvent::GestureSwipeUpdate { event } => gestures::handle_swipe_update::<B>(state, event),
        InputEvent::GestureSwipeEnd { event } => gestures::handle_swipe_end::<B>(state, event),
        _ => {}
    }
}
