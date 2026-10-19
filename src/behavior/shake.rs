//! Spots the mouse pointer being shaken: quick back-and-forth moves.

use std::collections::VecDeque;

/// Only moves this recent count.
const WINDOW_SECONDS: f64 = 0.8;
/// A swing this long (points) one way, then back, is one change of direction.
const STEP: f64 = 25.0;
/// Changes of direction within the window that make a shake: quicker than
/// the back-and-forth of petting.
const MIN_REVERSALS: usize = 4;

#[derive(Default)]
pub struct Shake {
    /// (time, x) of the pointer, oldest first.
    samples: VecDeque<(f64, f64)>,
}

impl Shake {
    /// The pointer was at `x` at time `t` (seconds, increasing).
    pub fn add(&mut self, t: f64, x: f64) {
        self.samples.push_back((t, x));
        while self
            .samples
            .front()
            .is_some_and(|&(first, _)| t - first > WINDOW_SECONDS)
        {
            self.samples.pop_front();
        }
    }

    pub fn is_shaking(&self) -> bool {
        reversals(self.samples.iter().map(|&(_, x)| x)) >= MIN_REVERSALS
    }
}

/// Counts direction changes, ignoring jitter smaller than `STEP`.
fn reversals(xs: impl Iterator<Item = f64>) -> usize {
    let mut xs = xs.peekable();
    let Some(&first) = xs.peek() else { return 0 };
    // The furthest point reached in the current direction (0 until one is set).
    let (mut extreme, mut direction, mut count) = (first, 0.0, 0);
    for x in xs {
        let from_extreme = x - extreme;
        if direction == 0.0 {
            if from_extreme.abs() >= STEP {
                direction = from_extreme.signum();
                extreme = x;
            }
        } else if from_extreme * direction > 0.0 {
            extreme = x;
        } else if from_extreme.abs() >= STEP {
            count += 1;
            direction = -direction;
            extreme = x;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shake(xs: &[f64], dt: f64) -> Shake {
        let mut shake = Shake::default();
        for (i, &x) in xs.iter().enumerate() {
            shake.add(i as f64 * dt, x);
        }
        shake
    }

    #[test]
    fn back_and_forth_is_a_shake() {
        let xs = [0.0, 40.0, 0.0, 40.0, 0.0, 40.0];
        assert!(shake(&xs, 0.1).is_shaking());
    }

    #[test]
    fn a_petting_stroke_is_not() {
        // Two strokes a second, back and forth over the cat.
        let xs = [0.0, 30.0, 60.0, 30.0, 0.0, 30.0, 60.0];
        assert!(!shake(&xs, 0.125).is_shaking());
    }

    #[test]
    fn a_straight_move_or_jitter_is_not() {
        assert!(!shake(&[0.0, 50.0, 100.0, 150.0, 200.0], 0.1).is_shaking());
        assert!(!shake(&[0.0, 5.0, 0.0, 5.0, 0.0, 5.0, 0.0], 0.1).is_shaking());
    }

    #[test]
    fn slow_waves_are_forgotten() {
        let xs = [0.0, 40.0, 0.0, 40.0, 0.0, 40.0];
        assert!(!shake(&xs, 0.5).is_shaking());
    }
}
