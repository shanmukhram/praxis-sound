//! Planar stereo audio buffer shared by every crate.

#[derive(Clone, Debug, PartialEq, Default)]
pub struct StereoBuffer {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

impl StereoBuffer {
    pub fn silent(len: usize) -> StereoBuffer {
        StereoBuffer {
            left: vec![0.0; len],
            right: vec![0.0; len],
        }
    }

    pub fn len(&self) -> usize {
        self.left.len()
    }

    pub fn is_empty(&self) -> bool {
        self.left.is_empty()
    }

    /// Adds `other` into `self` starting at `offset`, ignoring samples past the end.
    pub fn mix_from(&mut self, other: &StereoBuffer, offset: usize, gain: f32) {
        let n = other.len().min(self.len().saturating_sub(offset));
        for i in 0..n {
            self.left[offset + i] += other.left[i] * gain;
            self.right[offset + i] += other.right[i] * gain;
        }
    }

    pub fn peak_abs(&self) -> f32 {
        self.left
            .iter()
            .chain(self.right.iter())
            .fold(0.0f32, |m, v| m.max(v.abs()))
    }

    /// Interleaved L R L R copy, for encoders and devices.
    pub fn interleaved(&self) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.len() * 2);
        for i in 0..self.len() {
            out.push(self.left[i]);
            out.push(self.right[i]);
        }
        out
    }

    /// Builds a stereo buffer from interleaved data; mono is duplicated, channels > 2 keep 0 and 1.
    pub fn from_interleaved(data: &[f32], channels: usize) -> StereoBuffer {
        let ch = channels.max(1);
        let frames = data.len() / ch;
        let mut b = StereoBuffer::silent(frames);
        for f in 0..frames {
            let l = data[f * ch];
            let r = if ch > 1 { data[f * ch + 1] } else { l };
            b.left[f] = l;
            b.right[f] = r;
        }
        b
    }
}
