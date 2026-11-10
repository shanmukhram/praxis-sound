//! Gain (kind 10). Parameters: 0 gain_db (dB).

use super::ramp::{RAMP_SECONDS, Ramp, ramp_len};
use crate::traits::Processor;
use crate::util::{db_to_gain, sanitize};

const KIND: usize = 10;

#[derive(Clone, Debug)]
pub struct GainFx {
    params: [f32; 1],
    gain: Ramp,
}

impl GainFx {
    pub fn new(sample_rate: f32, max_block: usize) -> GainFx {
        let params = super::defaults::<1>(KIND);
        let mut g = GainFx {
            params,
            gain: Ramp::new(db_to_gain(params[0]), 1),
        };
        g.prepare(sample_rate, max_block);
        g
    }
}

impl Processor for GainFx {
    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.gain = Ramp::new(
            db_to_gain(self.params[0]),
            ramp_len(RAMP_SECONDS, sample_rate),
        );
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let g = self.gain.next();
            *l = sanitize(*l * g);
            *r = sanitize(*r * g);
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if let Some(v) = super::clamp_param(KIND, index, value) {
            self.params[index] = v;
            self.gain.set(db_to_gain(v));
        }
    }

    fn reset(&mut self) {
        self.gain.snap();
    }
}
