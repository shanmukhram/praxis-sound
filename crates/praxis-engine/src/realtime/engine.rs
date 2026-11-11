//! The audio-callback side: commands in, audio out, status out.

use super::player::LoopRange;
use super::prepare::PreparedGraph;
use super::{
    CHUNK, Command, MAX_COMMANDS_PER_CALLBACK, Status, declick_samples, swap_crossfade_samples,
};
use praxis_dsp::util::{db_to_gain, sanitize};
use rtrb::{Consumer, Producer};

/// What happens when a declick fade-out ends.
#[derive(Clone, Copy, Debug, PartialEq)]
enum After {
    /// Stop; the next `Play` starts at the given position, else where the fade ended.
    Stop(Option<u64>),
    Seek(u64),
}

#[derive(Clone, Copy, Debug)]
struct Fade {
    remaining: usize,
    total: usize,
    then: After,
}

/// The real-time engine, owned by the audio callback. Create it with
/// [`super::RealtimeEngine::new`]; drive it with [`RealtimeEngine::process`].
pub struct RealtimeEngine {
    sample_rate: u32,
    commands: Consumer<Command>,
    reports: Producer<Status>,
    retired: Producer<Box<PreparedGraph>>,
    current: Option<Box<PreparedGraph>>,
    incoming: Option<Box<PreparedGraph>>,
    /// Graphs waiting for room in the return queue (never dropped here).
    parked: [Option<Box<PreparedGraph>>; 2],
    xfade_pos: usize,
    xfade_len: usize,
    declick_len: usize,
    playing: bool,
    fade: Option<Fade>,
    /// Where `Play` starts when stopped.
    position: u64,
    loop_range: Option<LoopRange>,
    ended: bool,
    scratch_l: Vec<f32>,
    scratch_r: Vec<f32>,
    callbacks: u64,
    frames: u64,
    swaps: u64,
    rejected_swaps: u64,
    dropped_reports: u64,
    xruns: u64,
    late_callbacks: u64,
}

impl std::fmt::Debug for RealtimeEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RealtimeEngine")
            .field("sample_rate", &self.sample_rate)
            .field("playing", &self.playing)
            .field("position", &self.position)
            .finish_non_exhaustive()
    }
}

impl RealtimeEngine {
    pub(super) fn from_parts(
        sample_rate: u32,
        commands: Consumer<Command>,
        reports: Producer<Status>,
        retired: Producer<Box<PreparedGraph>>,
    ) -> RealtimeEngine {
        RealtimeEngine {
            sample_rate,
            commands,
            reports,
            retired,
            current: None,
            incoming: None,
            parked: [None, None],
            xfade_pos: 0,
            xfade_len: swap_crossfade_samples(sample_rate),
            declick_len: declick_samples(sample_rate),
            playing: false,
            fade: None,
            position: 0,
            loop_range: None,
            ended: false,
            scratch_l: vec![0.0; CHUNK],
            scratch_r: vec![0.0; CHUNK],
            callbacks: 0,
            frames: 0,
            swaps: 0,
            rejected_swaps: 0,
            dropped_reports: 0,
            xruns: 0,
            late_callbacks: 0,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Device-side counters (underruns and late callbacks measured by the host), carried out in
    /// every [`Status`]. The engine itself cannot see the device clock.
    pub fn set_device_counters(&mut self, xruns: u64, late_callbacks: u64) {
        self.xruns = xruns;
        self.late_callbacks = late_callbacks;
    }

    /// Renders the next `left.len()` frames (any size; `left` and `right` equal length).
    /// Applies queued commands first. Never allocates, locks, blocks or panics.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let total = left.len().min(right.len());
        self.callbacks += 1;
        self.try_retire();
        self.apply_commands();
        let mut i = 0usize;
        while i < total {
            let mut n = (total - i).min(CHUNK);
            if let Some(f) = &self.fade {
                n = n.min(f.remaining.max(1));
            }
            self.render_chunk(&mut left[i..i + n], &mut right[i..i + n]);
            i += n;
        }
        self.frames += total as u64;
        self.report(&left[..total], &right[..total]);
    }

    /// [`Self::process`] into an interleaved buffer of `channels` channels: channel 0 is left,
    /// channel 1 right, a mono device gets the average and further channels are silent.
    /// `scratch_*` must hold at least `frames` samples (preallocate them).
    pub fn process_interleaved(
        &mut self,
        out: &mut [f32],
        channels: usize,
        scratch_l: &mut [f32],
        scratch_r: &mut [f32],
    ) {
        let ch = channels.max(1);
        let frames = (out.len() / ch).min(scratch_l.len()).min(scratch_r.len());
        let (l, r) = (&mut scratch_l[..frames], &mut scratch_r[..frames]);
        self.process(l, r);
        for (f, frame) in out.chunks_exact_mut(ch).take(frames).enumerate() {
            if ch == 1 {
                frame[0] = 0.5 * (l[f] + r[f]);
            } else {
                frame[0] = l[f];
                frame[1] = r[f];
                frame[2..].fill(0.0);
            }
        }
        out[frames * ch..].fill(0.0);
    }

    fn try_retire(&mut self) {
        for slot in 0..self.parked.len() {
            if let Some(g) = self.parked[slot].take()
                && let Err(rtrb::PushError::Full(g)) = self.retired.push(g)
            {
                self.parked[slot] = Some(g);
            }
        }
    }

    /// Hands a graph back to the control thread to be dropped there (never dropped here). When
    /// the return queue is full it is parked and pushed on a later callback; swaps wait meanwhile,
    /// so at most one graph is ever parked.
    fn retire(&mut self, g: Box<PreparedGraph>) {
        let Err(rtrb::PushError::Full(g)) = self.retired.push(g) else {
            return;
        };
        match self.parked.iter_mut().find(|s| s.is_none()) {
            Some(slot) => *slot = Some(g),
            // Unreachable (see above). Leaking beats freeing on the audio thread.
            None => std::mem::forget(g),
        }
    }

    fn can_swap(&self) -> bool {
        self.incoming.is_none() && self.parked.iter().all(Option::is_none)
    }

    fn apply_commands(&mut self) {
        for _ in 0..MAX_COMMANDS_PER_CALLBACK {
            let is_blocked_swap = match self.commands.peek() {
                Ok(Command::Swap(_)) => !self.can_swap(),
                Ok(_) => false,
                Err(_) => return,
            };
            if is_blocked_swap {
                return;
            }
            let Ok(cmd) = self.commands.pop() else {
                return;
            };
            self.apply(cmd);
        }
    }

    fn musical_position(&self) -> u64 {
        match (&self.current, self.playing) {
            (Some(g), true) => g.cursor.out_pos,
            _ => self.position,
        }
    }

    /// Applies a command directly (control side, before the engine moves to the callback).
    pub(super) fn apply_now(&mut self, cmd: Command) {
        self.apply(cmd);
    }

    fn apply(&mut self, cmd: Command) {
        match cmd {
            Command::Play => {
                if let Some(f) = &mut self.fade {
                    if let After::Stop(at) = f.then {
                        // Resume where the stop began instead of stopping.
                        let here = self.current.as_ref().map(|g| g.cursor.out_pos);
                        f.then = After::Seek(at.or(here).unwrap_or(0));
                    }
                } else if !self.playing {
                    self.playing = true;
                    self.ended = false;
                    let p = self.position;
                    if let Some(g) = &mut self.current {
                        g.start_at(p);
                    }
                }
            }
            Command::Stop => {
                if self.playing {
                    self.begin_fade(After::Stop(None));
                }
            }
            Command::Seek(p) => {
                if self.playing {
                    match &mut self.fade {
                        Some(f) if matches!(f.then, After::Stop(_)) => {
                            f.then = After::Stop(Some(p))
                        }
                        Some(f) => f.then = After::Seek(p),
                        None => self.begin_fade(After::Seek(p)),
                    }
                } else {
                    self.position = p;
                }
            }
            Command::SetLoop(r) => {
                self.loop_range = r.filter(|r| r.start < r.end);
            }
            Command::SetLayerGain { layer, gain_db } => {
                let lin = if gain_db.is_finite() {
                    db_to_gain(gain_db.clamp(-60.0, 24.0))
                } else {
                    1.0
                };
                self.for_layer(layer, |l| l.trim = lin);
            }
            Command::SetLayerMute { layer, muted } => self.for_layer(layer, |l| l.muted = muted),
            Command::SetLayerSolo { layer, solo } => self.for_layer(layer, |l| l.solo = solo),
            Command::SetMasterGain { gain_db } => {
                if gain_db.is_finite() {
                    let lin = db_to_gain(gain_db.clamp(-60.0, 48.0));
                    for g in [&mut self.current, &mut self.incoming]
                        .into_iter()
                        .flatten()
                    {
                        g.gain_target = lin;
                    }
                }
            }
            Command::Swap(g) => self.swap(g),
        }
    }

    fn for_layer(&mut self, layer: usize, f: impl Fn(&mut super::prepare::LiveLayer)) {
        for g in [&mut self.current, &mut self.incoming]
            .into_iter()
            .flatten()
        {
            if let Some(&Some(n)) = g.node_of_layer.get(layer)
                && let Some(l) = g.live.get_mut(n)
            {
                f(l);
                g.update_solo();
            }
        }
    }

    fn begin_fade(&mut self, then: After) {
        if self.fade.is_none() {
            self.fade = Some(Fade {
                remaining: self.declick_len,
                total: self.declick_len,
                then,
            });
        }
    }

    fn swap(&mut self, mut g: Box<PreparedGraph>) {
        if g.sample_rate != self.sample_rate {
            self.rejected_swaps += 1;
            self.retire(g);
            return;
        }
        self.swaps += 1;
        match self.current.take() {
            Some(old) if self.playing => {
                g.start_at(old.cursor.out_pos);
                self.current = Some(old);
                self.incoming = Some(g);
                self.xfade_pos = 0;
            }
            Some(old) => {
                self.retire(old);
                self.current = Some(g);
            }
            None => {
                if self.playing {
                    g.start_at(self.position);
                }
                self.current = Some(g);
            }
        }
    }

    fn render_chunk(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len();
        if !self.playing || self.current.is_none() {
            left.fill(0.0);
            right.fill(0.0);
            return;
        }
        let lp = self.loop_range;
        if let Some(g) = &mut self.current {
            g.pull(left, right, lp);
        }
        // Crossfade into a swapped-in graph.
        if let Some(inc) = &mut self.incoming {
            let (sl, sr) = (&mut self.scratch_l[..n], &mut self.scratch_r[..n]);
            inc.pull(sl, sr, lp);
            let len = self.xfade_len.max(1);
            for k in 0..n {
                let w = (self.xfade_pos.min(len)) as f32 / len as f32;
                left[k] = sanitize(left[k] * (1.0 - w) + sl[k] * w);
                right[k] = sanitize(right[k] * (1.0 - w) + sr[k] * w);
                self.xfade_pos += 1;
            }
            if self.xfade_pos >= len {
                self.finish_crossfade();
            }
        }
        // Declick fade-out before a stop or a seek.
        if let Some(mut f) = self.fade {
            let total = f.total.max(1);
            for k in 0..n.min(f.remaining) {
                let w = (f.remaining - k) as f32 / total as f32;
                left[k] *= w;
                right[k] *= w;
            }
            let done = n.min(f.remaining);
            left[done..].fill(0.0);
            right[done..].fill(0.0);
            f.remaining -= done;
            if f.remaining == 0 {
                self.fade = None;
                self.finish_crossfade();
                match f.then {
                    After::Stop(at) => {
                        self.position = at.unwrap_or_else(|| self.musical_position());
                        self.playing = false;
                    }
                    After::Seek(p) => {
                        if let Some(g) = &mut self.current {
                            g.start_at(p);
                        }
                    }
                }
            } else {
                self.fade = Some(f);
            }
        }
        // A one-shot that played to its end stops; the next Play starts from the top.
        if let Some(g) = &self.current
            && g.cursor.ended
            && self.incoming.is_none()
            && self.fade.is_none()
            && self.playing
        {
            self.playing = false;
            self.ended = true;
            self.position = 0;
        }
    }

    /// Completes a running crossfade at once (before a stop or a seek).
    fn finish_crossfade(&mut self) {
        let Some(new) = self.incoming.take() else {
            return;
        };
        if let Some(old) = self.current.replace(new) {
            self.retire(old);
        }
    }

    fn report(&mut self, left: &[f32], right: &[f32]) {
        let mut peak = [0.0f32; 2];
        let mut sum = [0.0f64; 2];
        for (&l, &r) in left.iter().zip(right) {
            peak[0] = peak[0].max(l.abs());
            peak[1] = peak[1].max(r.abs());
            sum[0] += (l as f64) * (l as f64);
            sum[1] += (r as f64) * (r as f64);
        }
        let n = left.len().max(1) as f64;
        let status = Status {
            position: self.musical_position(),
            playing: self.playing,
            ended: self.ended,
            peak,
            rms: [(sum[0] / n).sqrt() as f32, (sum[1] / n).sqrt() as f32],
            callbacks: self.callbacks,
            frames: self.frames,
            xruns: self.xruns,
            late_callbacks: self.late_callbacks,
            swaps: self.swaps,
            rejected_swaps: self.rejected_swaps,
            crossfading: self.incoming.is_some(),
            dropped_reports: self.dropped_reports,
        };
        if self.reports.push(status).is_err() {
            self.dropped_reports += 1;
        }
    }
}
