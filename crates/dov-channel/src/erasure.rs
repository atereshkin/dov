//! Gilbert–Elliott frame-erasure model.
//!
//! Two states: Good (frames arrive) and Bad (frames are lost). Real cellular
//! loss is bursty, which matters a lot — a burst drives the decoder into muting,
//! which is qualitatively worse than the same number of scattered losses.

use crate::rng::Rng;

#[derive(Clone, Copy, PartialEq)]
enum State {
    Good,
    Bad,
}

#[derive(Clone)]
pub struct Erasure {
    state: State,
    /// P(Good → Bad) per frame.
    p_gb: f64,
    /// P(Bad → Good) per frame.
    p_bg: f64,
    /// Loss probability while in each state.
    loss_good: f64,
    loss_bad: f64,
}

impl Erasure {
    /// Memoryless loss at probability `p` (each frame independent).
    pub fn bernoulli(p: f64) -> Self {
        Self {
            state: State::Good,
            p_gb: 0.0,
            p_bg: 1.0,
            loss_good: p,
            loss_bad: p,
        }
    }

    /// Bursty loss: average loss fraction `avg_loss`, mean burst length
    /// `mean_burst` frames (lost in bursts, clean in between).
    pub fn bursty(avg_loss: f64, mean_burst: f64) -> Self {
        let p_bg = 1.0 / mean_burst.max(1.0);
        // stationary P(Bad) = p_gb / (p_gb + p_bg) = avg_loss  ⇒  solve for p_gb
        let p_gb = p_bg * avg_loss / (1.0 - avg_loss).max(1e-6);
        Self {
            state: State::Good,
            p_gb,
            p_bg,
            loss_good: 0.0,
            loss_bad: 1.0,
        }
    }

    /// Advance one frame; return true if this frame is erased.
    pub fn tick(&mut self, rng: &mut Rng) -> bool {
        // Transition first, then decide loss in the new state.
        self.state = match self.state {
            State::Good if rng.uniform() < self.p_gb => State::Bad,
            State::Bad if rng.uniform() < self.p_bg => State::Good,
            s => s,
        };
        let loss = match self.state {
            State::Good => self.loss_good,
            State::Bad => self.loss_bad,
        };
        rng.uniform() < loss
    }
}
