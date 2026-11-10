//! Small conversions shared everywhere.

pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

pub fn gain_to_db(g: f32) -> f32 {
    20.0 * g.abs().max(1e-12).log10()
}

pub fn db_to_gain_f64(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// Flushes denormals and NaN to zero.
#[inline]
pub fn sanitize(x: f32) -> f32 {
    if x.is_finite() && x.abs() > 1e-30 {
        x
    } else {
        0.0
    }
}

/// Equal-power pan law: pan -1..1 -> (left gain, right gain), both 1/sqrt(2) at centre.
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let p = (pan.clamp(-1.0, 1.0) + 1.0) * 0.25 * std::f32::consts::PI;
    (p.cos(), p.sin())
}

/// One-pole smoothing coefficient for a time constant in seconds.
pub fn time_coeff(seconds: f32, sample_rate: f32) -> f32 {
    if seconds <= 0.0 {
        0.0
    } else {
        (-1.0 / (seconds * sample_rate)).exp()
    }
}
