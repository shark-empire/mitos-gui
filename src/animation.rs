//! Small animation primitives for the MITOS shell.
//!
//! Stage 3 starts with the reusable timing/interpolation pieces.
//! Rendering code should consume these values rather than owning
//! animation timing itself.

use std::time::{Duration, Instant};

/// Simple normalized animation progress.
///
/// `0.0` = beginning
/// `1.0` = finished
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress(pub f32);

impl Progress {
    pub const ZERO: Self = Self(0.0);
    pub const ONE: Self = Self(1.0);

    pub fn clamp(self) -> Self {
        Self(self.0.clamp(0.0, 1.0))
    }

    /// Smooth cubic ease-in/ease-out.
    pub fn ease_in_out(self) -> Self {
        let t = self.clamp().0;
        Self(t * t * (3.0 - 2.0 * t))
    }

    /// Snappier ease-out with a slight overshoot before settling — reads
    /// as something "arriving with intent" or "powering on" rather than
    /// just fading in. Standard easeOutBack (see easings.net); can run
    /// briefly above `1.0` mid-curve by design (the overshoot) before
    /// landing exactly on `1.0` — callers driving a size/extent from this
    /// should expect that and clamp if they need a hard ceiling.
    pub fn ease_out_back(self) -> Self {
        let t = self.clamp().0;
        const C1: f32 = 1.70158;
        const C3: f32 = C1 + 1.0;
        let t1 = t - 1.0;
        Self(1.0 + C3 * t1 * t1 * t1 + C1 * t1 * t1)
    }

    pub fn is_finished(self) -> bool {
        self.0 >= 1.0
    }
}

/// A simple time-based animation.
#[derive(Clone, Copy, Debug)]
pub struct Animation {
    started: Instant,
    duration: Duration,
}

impl Animation {
    pub fn new(duration: Duration) -> Self {
        Self {
            started: Instant::now(),
            duration,
        }
    }

    pub fn progress(&self, now: Instant) -> Progress {
        if self.duration.is_zero() {
            return Progress::ONE;
        }

        let elapsed = now.saturating_duration_since(self.started);

        if elapsed.is_zero() {
            return Progress::ZERO;
        }

        Progress((elapsed.as_secs_f32() / self.duration.as_secs_f32()).min(1.0))
    }

    pub fn finished(&self, now: Instant) -> bool {
        self.progress(now).is_finished()
    }
}

/// Interpolates between two values.
pub fn lerp(from: f32, to: f32, progress: Progress) -> f32 {
    from + (to - from) * progress.clamp().0
}
