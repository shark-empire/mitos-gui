//! Stage 7: Accessibility input filters -- sticky keys, slow keys,
//! bounce keys.
//!
//! Scope, stated plainly: these filters decide which raw key events
//! make it through to the rest of the compositor (`keyboard.rs`), and
//! -- for sticky keys -- which modifiers MITOS's *own* Super+ shortcuts
//! see. They do not, and cannot safely from here, alter the xkbcommon
//! modifier state used to translate keysyms for text forwarded to
//! Wayland clients: doing that would mean feeding synthetic press/
//! release pairs into xkb's internal state tracking in a way this
//! codebase has no verified-safe API for, and getting it wrong risks
//! a modifier key reading as permanently stuck. Concretely:
//!   - Sticky keys: latching Shift and pressing R fires
//!     Super+Shift+R (Reboot) without holding Shift -- but typing into
//!     a text field still needs Shift held, same as without this on.
//!   - Slow keys / bounce keys: these only gate *whether* a keypress
//!     is let through at all, never what it means once it is, so they
//!     apply everywhere -- MITOS shortcuts and client text alike.
//!
//! Reduced motion and high contrast live in `theme.rs` instead (they
//! extend the existing runtime-theme system rather than being a new
//! one), and the on-screen keyboard is its own module.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long a key must be held before slow-keys accepts it.
const SLOW_KEYS_THRESHOLD: Duration = Duration::from_millis(300);

/// How soon a repeat of the same key is rejected as an accidental bounce.
const BOUNCE_KEYS_THRESHOLD: Duration = Duration::from_millis(300);

/// Which of Shift/Ctrl/Alt/Super are currently sticky-latched: pressed
/// and released on their own (with no other key going down in
/// between), waiting for the next non-modifier key to apply to.
#[derive(Default, Clone, Copy, Debug)]
pub struct StickyLatch {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub logo: bool,
}

impl StickyLatch {
    pub fn is_empty(&self) -> bool {
        !(self.shift || self.ctrl || self.alt || self.logo)
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// A bare modifier key (no other key pressed while it was down)
    /// went down then up -- flip its latch. Toggling (not just
    /// setting) means pressing the same modifier twice in a row
    /// cancels the latch instead of being a no-op, which matches how
    /// sticky keys behaves elsewhere.
    pub fn toggle(&mut self, keysym: u32) {
        use smithay::input::keyboard::keysyms;
        match keysym {
            keysyms::KEY_Shift_L | keysyms::KEY_Shift_R => self.shift = !self.shift,
            keysyms::KEY_Control_L | keysyms::KEY_Control_R => self.ctrl = !self.ctrl,
            keysyms::KEY_Alt_L | keysyms::KEY_Alt_R => self.alt = !self.alt,
            keysyms::KEY_Super_L | keysyms::KEY_Super_R => self.logo = !self.logo,
            _ => {}
        }
    }

    /// Whether `keysym` is one of the four modifier keys this struct
    /// tracks (used by `keyboard.rs` to tell "bare modifier tap" apart
    /// from "modifier held as part of a chord").
    pub fn is_modifier_keysym(keysym: u32) -> bool {
        use smithay::input::keyboard::keysyms;
        matches!(
            keysym,
            keysyms::KEY_Shift_L
                | keysyms::KEY_Shift_R
                | keysyms::KEY_Control_L
                | keysyms::KEY_Control_R
                | keysyms::KEY_Alt_L
                | keysyms::KEY_Alt_R
                | keysyms::KEY_Super_L
                | keysyms::KEY_Super_R
        )
    }
}

/// Effective modifier state for MITOS's own shortcuts: physically-held
/// OR sticky-latched. See the module doc for what this does and
/// doesn't cover.
#[derive(Clone, Copy, Debug)]
pub struct EffectiveMods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub logo: bool,
}

impl EffectiveMods {
    pub fn compute(mods: &smithay::input::keyboard::ModifiersState, sticky: &StickyLatch) -> Self {
        Self {
            shift: mods.shift || sticky.shift,
            ctrl: mods.ctrl || sticky.ctrl,
            alt: mods.alt || sticky.alt,
            logo: mods.logo || sticky.logo,
        }
    }
}

/// What a raw key event should do next, per the slow-keys/bounce-keys
/// filter below.
pub enum FilterDecision {
    /// Let it through immediately, unchanged.
    Pass,
    /// Drop it -- e.g. a bounce-keys reject, or a slow-keys press
    /// that's only just started being timed.
    Swallow,
    /// Slow-keys just confirmed a press that was being timed (this
    /// event is the matching *release*, arriving after the hold
    /// threshold). The original keydown was swallowed while timing
    /// it, so nothing downstream has seen this key yet -- the caller
    /// should synthesize a fresh, properly-paired press-then-release
    /// through the normal path now, using the current moment for
    /// both, rather than try to retroactively resurrect the original
    /// keydown.
    ConfirmedHold,
}

/// Filters raw keydown/keyup events for slow-keys and bounce-keys
/// before they reach the rest of the compositor. Owns only the small
/// bit of per-key timing state these two need; everything about what
/// a keypress *means* is still decided downstream in `keyboard.rs`,
/// exactly as when both are off.
///
/// Generic over the key identifier type (`smithay`'s `Keycode`, in
/// practice) rather than assuming it's a bare `u32`: all this needs
/// from it is `Copy + Eq + Hash` to use as a map key/token, so there's
/// no need to depend on exactly how that type represents a keycode
/// internally.
pub struct KeyFilter<K> {
    /// Keycode currently being timed for slow-keys, and when it went down.
    pending: Option<(K, Instant)>,
    /// Last time each keycode was accepted, for bounce-keys.
    last_accepted: HashMap<K, Instant>,
    /// Keycodes bounce-keys is currently rejecting -- tracked so the
    /// matching key-up is swallowed too, not just the key-down (a
    /// rejected press must not leave a dangling "release" event
    /// reaching a key nothing ever saw go down).
    rejected_down: std::collections::HashSet<K>,
}

impl<K> Default for KeyFilter<K> {
    fn default() -> Self {
        Self {
            pending: None,
            last_accepted: HashMap::new(),
            rejected_down: std::collections::HashSet::new(),
        }
    }
}

impl<K: Copy + Eq + std::hash::Hash> KeyFilter<K> {
    /// `enabled_slow`/`enabled_bounce` are passed in per call rather
    /// than fixed at construction, so toggling either setting live
    /// takes effect on the very next keypress rather than needing a
    /// restart.
    pub fn filter(
        &mut self,
        keycode: K,
        pressed: bool,
        enabled_slow: bool,
        enabled_bounce: bool,
    ) -> FilterDecision {
        let now = Instant::now();

        if !pressed && self.rejected_down.remove(&keycode) {
            return FilterDecision::Swallow;
        }

        if enabled_bounce && pressed {
            if let Some(&last) = self.last_accepted.get(&keycode) {
                if now.duration_since(last) < BOUNCE_KEYS_THRESHOLD {
                    self.rejected_down.insert(keycode);
                    return FilterDecision::Swallow;
                }
            }
        }

        if enabled_slow {
            if pressed {
                // Start timing; the most recently pressed key is the
                // one being timed; if another was already pending it's
                // simply abandoned, same as a real keyboard only
                // tracks one "current" slow key at a time in practice.
                self.pending = Some((keycode, now));
                return FilterDecision::Swallow;
            }

            if let Some((pending_code, pressed_at)) = self.pending {
                if pending_code == keycode {
                    self.pending = None;
                    if now.duration_since(pressed_at) < SLOW_KEYS_THRESHOLD {
                        return FilterDecision::Swallow; // released too soon
                    }
                    if enabled_bounce {
                        self.last_accepted.insert(keycode, now);
                    }
                    return FilterDecision::ConfirmedHold;
                }
            }
        }

        if enabled_bounce && pressed {
            self.last_accepted.insert(keycode, now);
        }

        FilterDecision::Pass
    }
}
