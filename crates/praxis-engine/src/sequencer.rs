//! Per-layer note sequencer: turns expanded notes into a sorted stream of note-on / note-off
//! events at exact sample positions.
//!
//! For loops the event list covers one period `[0, L)` and repeats forever. A note-off (or, in
//! principle, a note-on) that falls past the end of the loop is *folded*: it is stored at
//! `pos = t % L` with `lag = t / L`, and is emitted in cycle `c` only if `c >= lag` (it belongs to
//! the note of cycle `c - lag`). That is exactly what a looping player does, so tails of the last
//! bar ring into the first bar of the next cycle. For one-shots positions are absolute and nothing
//! repeats.
//!
//! Real-time rules: everything is built in [`Sequencer::new`]; [`Sequencer::peek_time`] and
//! [`Sequencer::pop`] never allocate, and their loops are bounded by the event count.

use crate::instrument::NoteOn;
use crate::seeding;
use praxis_score::Accent;
use praxis_score::timeline::NoteEvent;

/// One scheduled event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SeqEventKind {
    /// Note-off of note `note` (index into the layer's note list).
    Off,
    /// Note-on of note `note`.
    On,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SeqEvent {
    /// Position within the period (loops) or absolute sample (one-shots).
    pos: u64,
    /// Number of whole periods between the note's cycle and the cycle the event sounds in.
    lag: u64,
    kind: SeqEventKind,
    note: u32,
}

/// The note-on data of one note, without the per-cycle id.
#[derive(Clone, Copy, Debug, PartialEq)]
struct NoteTemplate {
    pitch: u8,
    velocity: f32,
    brightness: f32,
    seed: u64,
    length_samples: u64,
}

/// An event ready to deliver to an instrument.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Delivered {
    On(NoteOn),
    Off(u64),
}

/// Sorted event stream for one layer.
#[derive(Clone, Debug)]
pub struct Sequencer {
    events: Vec<SeqEvent>,
    notes: Vec<NoteTemplate>,
    period: Option<u64>,
    min_lag: u64,
    cycle: u64,
    index: usize,
    /// Latest note-off position relative to the start of a note's own cycle (for tail planning).
    last_off: u64,
}

impl Sequencer {
    /// Builds the event stream for one layer from its expanded notes (in `expand_notes` order).
    /// `period` is the loop length in samples for loops, `None` for one-shots.
    pub fn new(
        notes: &[&NoteEvent],
        period: Option<u64>,
        score_seed: u64,
        seed_offset: u64,
    ) -> Sequencer {
        let period = period.filter(|p| *p > 0);
        let mut templates = Vec::with_capacity(notes.len());
        let mut events = Vec::with_capacity(notes.len() * 2);
        let mut last_off = 0u64;
        for (i, n) in notes.iter().enumerate() {
            templates.push(NoteTemplate {
                pitch: n.pitch,
                velocity: n.velocity,
                brightness: if n.accent == Accent::Bright { 1.0 } else { 0.0 },
                seed: seeding::note_seed(score_seed, seed_offset, n.key),
                length_samples: n.length_samples,
            });
            let on = n.start_sample;
            let off = n.start_sample + n.length_samples.max(1);
            last_off = last_off.max(off);
            for (t, kind) in [(on, SeqEventKind::On), (off, SeqEventKind::Off)] {
                let (pos, lag) = match period {
                    Some(p) => (t % p, t / p),
                    None => (t, 0),
                };
                events.push(SeqEvent {
                    pos,
                    lag,
                    kind,
                    note: i as u32,
                });
            }
        }
        // Offs before ons at the same sample, so a repeated pitch releases before it restrikes.
        events.sort_by(|a, b| {
            let ka = (a.pos, a.kind == SeqEventKind::On, a.note, a.lag);
            let kb = (b.pos, b.kind == SeqEventKind::On, b.note, b.lag);
            ka.cmp(&kb)
        });
        let min_lag = events.iter().map(|e| e.lag).min().unwrap_or(0);
        Sequencer {
            events,
            notes: templates,
            period,
            min_lag,
            cycle: 0,
            index: 0,
            last_off,
        }
    }

    /// Number of notes per cycle.
    pub fn note_count(&self) -> usize {
        self.notes.len()
    }

    /// Samples by which notes outlast the end of the musical length `len` (0 if none do).
    pub fn overhang(&self, len: u64) -> u64 {
        self.last_off.saturating_sub(len)
    }

    /// Moves the cursor onto the next event that may sound. Bounded: at most one pass over the
    /// list per cycle up to `min_lag + 1`.
    fn normalize(&mut self) -> bool {
        if self.events.is_empty() {
            return false;
        }
        if self.period.is_some() && self.cycle < self.min_lag {
            self.cycle = self.min_lag;
            self.index = 0;
        }
        let mut guard = 0usize;
        let limit = self.events.len() * 2 + 2;
        loop {
            if self.index >= self.events.len() {
                if self.period.is_none() {
                    return false;
                }
                self.cycle += 1;
                self.index = 0;
            }
            if self.events[self.index].lag <= self.cycle {
                return true;
            }
            self.index += 1;
            guard += 1;
            if guard > limit {
                return false;
            }
        }
    }

    /// Absolute sample of the next event, if any.
    pub fn peek_time(&mut self) -> Option<u64> {
        if !self.normalize() {
            return None;
        }
        let e = self.events[self.index];
        Some(match self.period {
            Some(p) => self.cycle * p + e.pos,
            None => e.pos,
        })
    }

    /// Takes the next event. Call after [`Sequencer::peek_time`] returned its time.
    pub fn pop(&mut self) -> Option<Delivered> {
        if !self.normalize() {
            return None;
        }
        let e = self.events[self.index];
        self.index += 1;
        let note_cycle = self.cycle - e.lag;
        let id = e.note as u64 + note_cycle * self.notes.len() as u64;
        Some(match e.kind {
            SeqEventKind::Off => Delivered::Off(id),
            SeqEventKind::On => {
                let t = self.notes[e.note as usize];
                Delivered::On(NoteOn {
                    id,
                    pitch: t.pitch,
                    velocity: t.velocity,
                    brightness: t.brightness,
                    seed: t.seed,
                    length_samples: t.length_samples,
                })
            }
        })
    }

    /// Rewinds to time zero.
    pub fn rewind(&mut self) {
        self.cycle = 0;
        self.index = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start: u64, len: u64, pitch: u8) -> NoteEvent {
        NoteEvent {
            layer: 0,
            key: pitch as u64,
            bar: 1,
            start_s: 0.0,
            length_s: 0.0,
            start_sample: start,
            length_samples: len,
            pitch,
            velocity: 0.5,
            accent: Accent::Normal,
        }
    }

    fn drain(seq: &mut Sequencer, until: u64) -> Vec<(u64, Delivered)> {
        let mut out = Vec::new();
        while let Some(t) = seq.peek_time() {
            if t >= until {
                break;
            }
            out.push((t, seq.pop().unwrap()));
        }
        out
    }

    #[test]
    fn loop_folds_offs_past_the_end() {
        let a = note(0, 10, 60);
        let b = note(90, 30, 62); // off at 120 -> pos 20 of the next cycle
        let mut seq = Sequencer::new(&[&a, &b], Some(100), 1, 0);
        let ev = drain(&mut seq, 300);
        let times: Vec<u64> = ev.iter().map(|e| e.0).collect();
        // cycle 0: on a@0, off a@10, on b@90; cycle 1: on a@100, off a@110, off b(cycle0)@120 ...
        assert_eq!(
            times,
            vec![0, 10, 90, 100, 110, 120, 190, 200, 210, 220, 290]
        );
        match ev[5].1 {
            Delivered::Off(id) => assert_eq!(id, 1), // note b of cycle 0
            _ => panic!("expected off"),
        }
        match ev[6].1 {
            Delivered::On(n) => assert_eq!(n.id, 3), // note b of cycle 1
            _ => panic!("expected on"),
        }
    }

    #[test]
    fn one_shot_ends() {
        let a = note(5, 10, 60);
        let mut seq = Sequencer::new(&[&a], None, 1, 0);
        assert_eq!(drain(&mut seq, u64::MAX).len(), 2);
        assert_eq!(seq.peek_time(), None);
        let mut empty = Sequencer::new(&[], Some(100), 1, 0);
        assert_eq!(empty.peek_time(), None);
    }

    #[test]
    fn offs_sort_before_ons_at_the_same_sample() {
        let a = note(0, 10, 60);
        let b = note(10, 10, 60);
        let mut seq = Sequencer::new(&[&a, &b], None, 1, 0);
        let ev = drain(&mut seq, u64::MAX);
        assert!(matches!(ev[1], (10, Delivered::Off(0))));
        assert!(matches!(ev[2], (10, Delivered::On(_))));
    }
}
