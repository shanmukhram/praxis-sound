//! Envelope generators: [`Adsr`], one-shot [`Ar`] and SFZ-style [`Dahdsr`].
//!
//! Shared behaviour:
//! - Times are in seconds, levels are linear gain `0.0..=1.0`, the peak is 1.0.
//! - `next()` advances one sample and returns the new level. A stage of `t` seconds lasts
//!   `round(t * sample_rate)` samples: after `gate(true)` from idle, the `round(attack * sr)`-th
//!   call returns exactly 1.0.
//! - Attack is linear at a constant rate: a full attack takes the attack time, and a retrigger
//!   from a level `L` rises from `L` (never restarting from 0) and takes `(1 - L) * attack`.
//!   Retriggering therefore never jumps.
//! - Decay and release are exponential and reach their end level in exactly their time: they
//!   aim past the end point by an overshoot set by the curve (see [`Adsr::set_curve`]), so the
//!   tail never lingers forever. Release takes the release time from whatever level it starts.
//! - A sustain level of 0 ends the envelope (idle) when the decay finishes, which lets voices
//!   free themselves for plucked and struck sounds.
//! - Parameter changes apply from the next stage that starts; a sustain change during the
//!   sustain stage glides there over a few milliseconds instead of jumping.

// `next()` returning a bare level is the crate-wide envelope contract.
#![allow(clippy::should_implement_trait)]

/// Envelope stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Idle,
    Delay,
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
}

/// Overshoot ratio for a curve amount `0..=1`: 100 (nearly straight) at 0, 0.1 at 0.5 and
/// 1e-4 (exponential down to -80 dB, which sounds natural) at 1.
fn overshoot_for(curve: f32) -> f64 {
    let c = if curve.is_finite() {
        curve.clamp(0.0, 1.0) as f64
    } else {
        1.0
    };
    10f64.powf(2.0 - 6.0 * c)
}

/// Level treated as silence when a zero sustain ends a note (-120 dB).
const SILENT: f64 = 1e-6;

fn seconds(t: f32) -> f32 {
    if t.is_finite() {
        t.clamp(0.0, 3600.0)
    } else {
        0.0
    }
}

/// Delay / attack / hold / decay / sustain / release envelope with an optional start level,
/// following the SFZ `ampeg_*` opcodes (`ampeg_start` is [`Dahdsr::set_start`], in `0..=1`
/// rather than percent).
#[derive(Clone, Debug)]
pub struct Dahdsr {
    sr: f64,
    delay: f32,
    attack: f32,
    hold: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    start: f32,
    decay_overshoot: f64,
    release_overshoot: f64,
    sustain_glide: f64,
    stage: Stage,
    level: f64,
    remaining: u64,
    inc: f64,
    coeff: f64,
    target: f64,
    end_level: f64,
}

impl Dahdsr {
    /// Defaults: no delay or hold, 5 ms attack, 200 ms decay, sustain 0.7, 300 ms release,
    /// curve 1 (natural exponential).
    pub fn new(sample_rate: f32) -> Dahdsr {
        let sr = if sample_rate.is_finite() && sample_rate > 0.0 {
            sample_rate as f64
        } else {
            48_000.0
        };
        Dahdsr {
            sr,
            delay: 0.0,
            attack: 0.005,
            hold: 0.0,
            decay: 0.2,
            sustain: 0.7,
            release: 0.3,
            start: 0.0,
            decay_overshoot: overshoot_for(1.0),
            release_overshoot: overshoot_for(1.0),
            sustain_glide: 1.0 - (-1.0 / (0.004 * sr)).exp(),
            stage: Stage::Idle,
            level: 0.0,
            remaining: 0,
            inc: 0.0,
            coeff: 1.0,
            target: 0.0,
            end_level: 0.0,
        }
    }

    /// Delay before the attack, seconds.
    pub fn set_delay(&mut self, s: f32) {
        self.delay = seconds(s);
    }
    /// Attack time from 0 to 1, seconds (linear).
    pub fn set_attack(&mut self, s: f32) {
        self.attack = seconds(s);
    }
    /// Time held at the peak after the attack, seconds.
    pub fn set_hold(&mut self, s: f32) {
        self.hold = seconds(s);
    }
    /// Time from the peak to the sustain level, seconds.
    pub fn set_decay(&mut self, s: f32) {
        self.decay = seconds(s);
    }
    /// Sustain level `0..=1`.
    pub fn set_sustain(&mut self, level: f32) {
        self.sustain = if level.is_finite() {
            level.clamp(0.0, 1.0)
        } else {
            0.0
        };
    }
    /// Release time from the current level to 0, seconds.
    pub fn set_release(&mut self, s: f32) {
        self.release = seconds(s);
    }
    /// Level the attack starts from when triggered from idle, `0..=1`.
    pub fn set_start(&mut self, level: f32) {
        self.start = if level.is_finite() {
            level.clamp(0.0, 1.0)
        } else {
            0.0
        };
    }
    /// Curvature of decay and release, `0..=1` (see [`Adsr::set_curve`]).
    pub fn set_curve(&mut self, curve: f32) {
        self.set_decay_curve(curve);
        self.set_release_curve(curve);
    }
    pub fn set_decay_curve(&mut self, curve: f32) {
        self.decay_overshoot = overshoot_for(curve);
    }
    pub fn set_release_curve(&mut self, curve: f32) {
        self.release_overshoot = overshoot_for(curve);
    }

    fn samples(&self, s: f32) -> u64 {
        (s as f64 * self.sr).round() as u64
    }

    /// Note on (`true`) or off (`false`). Note on while active retriggers from the current level.
    pub fn gate(&mut self, on: bool) {
        if on {
            if self.stage == Stage::Idle {
                self.level = self.start as f64;
            }
            self.enter(Stage::Delay);
        } else if self.stage != Stage::Idle {
            self.enter(Stage::Release);
        }
    }

    /// Silences immediately.
    pub fn reset(&mut self) {
        self.stage = Stage::Idle;
        self.level = 0.0;
        self.remaining = 0;
    }

    pub fn is_idle(&self) -> bool {
        self.stage == Stage::Idle
    }

    pub fn stage(&self) -> Stage {
        self.stage
    }

    /// Current level without advancing.
    pub fn level(&self) -> f32 {
        self.level as f32
    }

    fn setup_exp(&mut self, from: f64, to: f64, n: u64, overshoot: f64) {
        self.remaining = n;
        self.end_level = to;
        self.target = to - overshoot * (from - to);
        self.coeff = (overshoot / (1.0 + overshoot)).powf(1.0 / n as f64);
    }

    fn enter(&mut self, stage: Stage) {
        let mut stage = stage;
        loop {
            match stage {
                Stage::Delay => {
                    let n = self.samples(self.delay);
                    if n == 0 {
                        stage = Stage::Attack;
                        continue;
                    }
                    self.remaining = n;
                }
                Stage::Attack => {
                    let n = ((1.0 - self.level).max(0.0) * self.attack as f64 * self.sr).round();
                    if n < 1.0 || self.level >= 1.0 {
                        self.level = 1.0;
                        stage = Stage::Hold;
                        continue;
                    }
                    self.remaining = n as u64;
                    self.inc = (1.0 - self.level) / n;
                }
                Stage::Hold => {
                    let n = self.samples(self.hold);
                    if n == 0 {
                        stage = Stage::Decay;
                        continue;
                    }
                    self.remaining = n;
                }
                Stage::Decay => {
                    let n = self.samples(self.decay);
                    let s = self.sustain as f64;
                    if n == 0 || self.level <= s {
                        self.level = s.min(self.level);
                        stage = Stage::Sustain;
                        continue;
                    }
                    self.setup_exp(self.level, s, n, self.decay_overshoot);
                }
                Stage::Sustain => {
                    // A decay into a zero sustain has already reached 0: the note is over.
                    if self.sustain <= 0.0 && self.level <= SILENT {
                        stage = Stage::Idle;
                        continue;
                    }
                }
                Stage::Release => {
                    let n = self.samples(self.release);
                    if n == 0 || self.level <= 0.0 {
                        self.level = 0.0;
                        stage = Stage::Idle;
                        continue;
                    }
                    self.setup_exp(self.level, 0.0, n, self.release_overshoot);
                }
                Stage::Idle => {
                    self.level = 0.0;
                    self.remaining = 0;
                }
            }
            break;
        }
        self.stage = stage;
    }

    /// Advances one sample and returns the level.
    #[inline]
    pub fn next(&mut self) -> f32 {
        match self.stage {
            Stage::Idle => {}
            Stage::Delay | Stage::Hold => {
                self.remaining -= 1;
                if self.remaining == 0 {
                    let next = if self.stage == Stage::Delay {
                        Stage::Attack
                    } else {
                        Stage::Decay
                    };
                    self.enter(next);
                }
            }
            Stage::Attack => {
                self.level += self.inc;
                self.remaining -= 1;
                if self.remaining == 0 {
                    self.level = 1.0;
                    self.enter(Stage::Hold);
                }
            }
            Stage::Decay | Stage::Release => {
                self.level = self.target + (self.level - self.target) * self.coeff;
                self.remaining -= 1;
                if self.remaining == 0 {
                    self.level = self.end_level;
                    let next = if self.stage == Stage::Decay {
                        Stage::Sustain
                    } else {
                        Stage::Idle
                    };
                    self.enter(next);
                }
            }
            Stage::Sustain => {
                // Glide to a changed sustain level; a sustain lowered to 0 glides out and then
                // frees the voice.
                let s = self.sustain as f64;
                let d = s - self.level;
                self.level = if d.abs() < 1e-7 {
                    s
                } else {
                    self.level + d * self.sustain_glide
                };
                if s <= 0.0 && self.level <= SILENT {
                    self.enter(Stage::Idle);
                }
            }
        }
        self.level as f32
    }
}

/// Attack / decay / sustain / release envelope (see the module docs for timing and curves).
#[derive(Clone, Debug)]
pub struct Adsr {
    inner: Dahdsr,
}

impl Adsr {
    /// Defaults: 5 ms attack, 200 ms decay, sustain 0.7, 300 ms release, curve 1.
    pub fn new(sample_rate: f32) -> Adsr {
        Adsr {
            inner: Dahdsr::new(sample_rate),
        }
    }

    /// All four parameters at once: seconds, seconds, level `0..=1`, seconds.
    pub fn set_params(&mut self, attack: f32, decay: f32, sustain: f32, release: f32) {
        self.inner.set_attack(attack);
        self.inner.set_decay(decay);
        self.inner.set_sustain(sustain);
        self.inner.set_release(release);
    }
    /// Attack time from 0 to 1, seconds (linear).
    pub fn set_attack(&mut self, s: f32) {
        self.inner.set_attack(s);
    }
    /// Time from the peak to the sustain level, seconds.
    pub fn set_decay(&mut self, s: f32) {
        self.inner.set_decay(s);
    }
    /// Sustain level `0..=1`.
    pub fn set_sustain(&mut self, level: f32) {
        self.inner.set_sustain(level);
    }
    /// Release time from the current level to 0, seconds.
    pub fn set_release(&mut self, s: f32) {
        self.inner.set_release(s);
    }
    /// Curvature of decay and release, `0..=1`: 0 is nearly straight, 0.5 moderately curved,
    /// 1 (default) exponential down to -80 dB (a straight line in dB, like a ringing string).
    pub fn set_curve(&mut self, curve: f32) {
        self.inner.set_curve(curve);
    }
    pub fn set_decay_curve(&mut self, curve: f32) {
        self.inner.set_decay_curve(curve);
    }
    pub fn set_release_curve(&mut self, curve: f32) {
        self.inner.set_release_curve(curve);
    }
    /// Note on / off. Note on while sounding retriggers from the current level.
    pub fn gate(&mut self, on: bool) {
        self.inner.gate(on);
    }
    /// Advances one sample and returns the level.
    #[inline]
    pub fn next(&mut self) -> f32 {
        self.inner.next()
    }
    pub fn is_idle(&self) -> bool {
        self.inner.is_idle()
    }
    pub fn level(&self) -> f32 {
        self.inner.level()
    }
    pub fn stage(&self) -> Stage {
        self.inner.stage()
    }
    /// Silences immediately.
    pub fn reset(&mut self) {
        self.inner.reset();
    }
}

/// One-shot attack / release envelope for drums and effects: a trigger rises linearly to 1 in
/// the attack time, then falls exponentially to 0 in the release time, regardless of note off.
#[derive(Clone, Debug)]
pub struct Ar {
    inner: Dahdsr,
}

impl Ar {
    /// Defaults: 1 ms attack, 300 ms release, curve 1.
    pub fn new(sample_rate: f32) -> Ar {
        let mut inner = Dahdsr::new(sample_rate);
        inner.set_attack(0.001);
        inner.set_decay(0.3);
        inner.set_sustain(0.0);
        Ar { inner }
    }

    /// Attack time, seconds (linear).
    pub fn set_attack(&mut self, s: f32) {
        self.inner.set_attack(s);
    }
    /// Time from the peak to silence, seconds.
    pub fn set_release(&mut self, s: f32) {
        self.inner.set_decay(s);
    }
    /// Curvature of the release, `0..=1` (see [`Adsr::set_curve`]).
    pub fn set_curve(&mut self, curve: f32) {
        self.inner.set_decay_curve(curve);
    }
    /// Starts the envelope (from the current level when already sounding).
    pub fn trigger(&mut self) {
        self.inner.gate(true);
    }
    /// `true` triggers; `false` is ignored (one-shot).
    pub fn gate(&mut self, on: bool) {
        if on {
            self.trigger();
        }
    }
    /// Advances one sample and returns the level.
    #[inline]
    pub fn next(&mut self) -> f32 {
        self.inner.next()
    }
    pub fn is_idle(&self) -> bool {
        self.inner.is_idle()
    }
    pub fn level(&self) -> f32 {
        self.inner.level()
    }
    pub fn stage(&self) -> Stage {
        self.inner.stage()
    }
    /// Silences immediately.
    pub fn reset(&mut self) {
        self.inner.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_times_are_instant_and_safe() {
        let mut e = Adsr::new(48_000.0);
        e.set_params(0.0, 0.0, 0.5, 0.0);
        e.gate(true);
        assert_eq!(e.next(), 0.5);
        e.gate(false);
        assert!(e.is_idle());
        assert_eq!(e.next(), 0.0);
    }

    #[test]
    fn sustain_zero_frees_the_voice() {
        let mut e = Adsr::new(48_000.0);
        e.set_params(0.001, 0.01, 0.0, 0.1);
        e.gate(true);
        for _ in 0..(48 + 480) {
            e.next();
        }
        assert!(e.is_idle());
    }
}
