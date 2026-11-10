//! Standard MIDI File export (FR-32). WP-E audio-io; see docs/contracts/audio-io.md.
//!
//! SMF type 1 at 480 PPQ. Track 0 is the conductor track (score title, tempo, time signature,
//! key signature, section markers and hit/cue markers). Then one track per layer, in layer
//! order, named after the layer; muted layers are exported too, with " (muted)" appended to
//! the name. Drum and percussion layers play on MIDI channel 10; other layers take channels
//! 1-16 in turn, skipping 10.
//!
//! Notes come from [`praxis_score::timeline::expand_notes`], so clip ranges, pattern offsets,
//! transposition, velocity scaling and accents are exactly what the engine renders. Their
//! start and length in seconds are converted back to ticks with the constant tempo; every grid
//! the Score supports is a whole number of ticks at 480 PPQ, so grid notes land exactly on
//! their ticks.

use midly::num::{u4, u7, u15, u24, u28};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};
use praxis_score::timeline::{TimeMap, expand_notes};
use praxis_score::{GeneratorClass, Key, Mode, Score};

/// Ticks per quarter note.
pub const PPQ: u16 = 480;
/// MIDI channel (0-based) used for drum layers: channel 10.
pub const DRUM_CHANNEL: u8 = 9;
/// Sample rate passed to the time map; only the seconds fields of the events are used.
const REFERENCE_RATE: u32 = 48_000;

/// Converts a Score to a Standard MIDI File.
pub fn score_to_midi(score: &Score) -> Vec<u8> {
    let tm = TimeMap::new(score, REFERENCE_RATE);
    let ticks_per_s = if score.tempo.bpm.is_finite() && score.tempo.bpm > 0.0 {
        score.tempo.bpm / 60.0 * PPQ as f64
    } else {
        0.0
    };
    let to_ticks = |s: f64| -> u64 {
        let t = (s * ticks_per_s).round();
        if t.is_finite() && t > 0.0 {
            t as u64
        } else {
            0
        }
    };
    let bar_ticks = |bar: f64| to_ticks(tm.bar_pos_to_s(bar));
    let score_end = bar_ticks(score.bars as f64 + 1.0);

    let names: Vec<Vec<u8>> = score
        .layers
        .iter()
        .map(|l| {
            let mut n = if l.name.trim().is_empty() {
                l.id.clone()
            } else {
                l.name.clone()
            };
            if l.muted {
                n.push_str(" (muted)");
            }
            n.into_bytes()
        })
        .collect();
    let presets: Vec<Vec<u8>> = score
        .layers
        .iter()
        .map(|l| l.instrument.preset.clone().into_bytes())
        .collect();
    let section_names: Vec<Vec<u8>> = score
        .sections
        .iter()
        .map(|s| {
            if s.name.trim().is_empty() {
                s.id.clone().into_bytes()
            } else {
                s.name.clone().into_bytes()
            }
        })
        .collect();
    let marker_names: Vec<Vec<u8>> = score
        .markers
        .iter()
        .map(|m| {
            if m.label.trim().is_empty() {
                m.id.clone().into_bytes()
            } else {
                m.label.clone().into_bytes()
            }
        })
        .collect();
    let title = if score.title.trim().is_empty() {
        b"Praxis Sound".to_vec()
    } else {
        score.title.clone().into_bytes()
    };

    // ---- conductor track ----
    let mut conductor: Vec<(u64, u8, TrackEventKind)> = vec![
        (0, 0, TrackEventKind::Meta(MetaMessage::TrackName(&title))),
        (
            0,
            1,
            TrackEventKind::Meta(MetaMessage::Tempo(u24::new(micros_per_quarter(
                score.tempo.bpm,
            )))),
        ),
        (
            0,
            2,
            TrackEventKind::Meta(time_signature(
                score.time_signature.numerator,
                score.time_signature.denominator,
            )),
        ),
    ];
    if let Some((sharps, minor)) = key_signature(score.key) {
        conductor.push((
            0,
            3,
            TrackEventKind::Meta(MetaMessage::KeySignature(sharps, minor)),
        ));
    }
    for (s, name) in score.sections.iter().zip(&section_names) {
        conductor.push((
            bar_ticks(s.bars.start.max(1) as f64),
            4,
            TrackEventKind::Meta(MetaMessage::Marker(name)),
        ));
    }
    for (m, name) in score.markers.iter().zip(&marker_names) {
        conductor.push((
            to_ticks(m.time_s),
            5,
            TrackEventKind::Meta(MetaMessage::CuePoint(name)),
        ));
    }

    let mut tracks = vec![finish_track(conductor, score_end)];

    // ---- one track per layer ----
    let notes = expand_notes(score, REFERENCE_RATE);
    let mut next_channel = 0u8;
    for (li, layer) in score.layers.iter().enumerate() {
        let channel = if matches!(
            layer.class,
            GeneratorClass::Drums | GeneratorClass::Percussion
        ) {
            DRUM_CHANNEL
        } else {
            let c = next_channel;
            next_channel = (next_channel + 1) % 16;
            if next_channel == DRUM_CHANNEL {
                next_channel += 1;
            }
            c
        };
        let ch = u4::new(channel);
        let mut events: Vec<(u64, u8, TrackEventKind)> = vec![
            (
                0,
                0,
                TrackEventKind::Meta(MetaMessage::TrackName(&names[li])),
            ),
            (
                0,
                1,
                TrackEventKind::Meta(MetaMessage::InstrumentName(&presets[li])),
            ),
        ];
        // (start, end, pitch, velocity), in start order.
        let mut spans: Vec<(u64, u64, u8, u8)> = notes
            .iter()
            .filter(|n| n.layer == li)
            .map(|n| {
                let start = to_ticks(n.start_s);
                let end = to_ticks(n.start_s + n.length_s).max(start + 1);
                let vel = (n.velocity as f64 * 127.0).round().clamp(1.0, 127.0) as u8;
                (start, end, n.pitch.min(127), vel)
            })
            .collect();
        spans.sort_by_key(|&(s, e, p, _)| (s, p, e));
        // A pitch cannot sound twice on one channel: a new note-on cuts the previous one.
        let mut sounding: [Option<usize>; 128] = [None; 128];
        for i in 0..spans.len() {
            let (start, _, pitch, _) = spans[i];
            if let Some(j) = sounding[pitch as usize]
                && spans[j].1 > start
            {
                spans[j].1 = start;
            }
            sounding[pitch as usize] = Some(i);
        }
        for &(start, end, pitch, vel) in &spans {
            if end <= start {
                continue; // fully replaced by a note at the same tick
            }
            let key = u7::new(pitch);
            events.push((
                start,
                3,
                TrackEventKind::Midi {
                    channel: ch,
                    message: MidiMessage::NoteOn {
                        key,
                        vel: u7::new(vel),
                    },
                },
            ));
            events.push((
                end,
                2, // note-offs before note-ons at the same tick
                TrackEventKind::Midi {
                    channel: ch,
                    message: MidiMessage::NoteOff {
                        key,
                        vel: u7::new(64),
                    },
                },
            ));
        }
        tracks.push(finish_track(events, score_end));
    }

    let mut smf = Smf::new(Header::new(
        Format::Parallel,
        Timing::Metrical(u15::new(PPQ)),
    ));
    smf.tracks = tracks;
    let mut out = Vec::new();
    // Writing to a Vec cannot fail for I/O reasons; midly only rejects impossible sizes.
    if smf.write_std(&mut out).is_err() {
        out.clear();
    }
    out
}

/// Sorts events by (tick, priority) and turns absolute ticks into deltas, ending the track at
/// the later of its last event and `end`.
fn finish_track<'a>(
    mut events: Vec<(u64, u8, TrackEventKind<'a>)>,
    end: u64,
) -> Vec<TrackEvent<'a>> {
    events.sort_by_key(|&(t, prio, ref kind)| (t, prio, note_of(kind)));
    let mut out = Vec::with_capacity(events.len() + 1);
    let mut last = 0u64;
    for (t, _, kind) in events {
        out.push(TrackEvent {
            delta: delta(t - last),
            kind,
        });
        last = t;
    }
    let end = end.max(last);
    out.push(TrackEvent {
        delta: delta(end - last),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    out
}

fn note_of(kind: &TrackEventKind) -> u8 {
    match kind {
        TrackEventKind::Midi {
            message: MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. },
            ..
        } => key.as_int(),
        _ => 0,
    }
}

fn delta(d: u64) -> u28 {
    u28::new(d.min(u28::max_value().as_int() as u64) as u32)
}

fn micros_per_quarter(bpm: f64) -> u32 {
    if !(bpm.is_finite() && bpm > 0.0) {
        return 500_000;
    }
    (60_000_000.0 / bpm)
        .round()
        .clamp(1.0, u24::max_value().as_int() as f64) as u32
}

fn time_signature(numerator: u8, denominator: u8) -> MetaMessage<'static> {
    let den_pow = denominator.max(1).trailing_zeros().min(7) as u8;
    // MIDI clocks (24 per quarter) per metronome click: one click per denominator beat.
    let clocks = (96u32 / denominator.max(1) as u32).clamp(1, 255) as u8;
    MetaMessage::TimeSignature(numerator, den_pow, clocks, 8)
}

/// Sharps (positive) or flats (negative) and the minor flag for the key, through the relative
/// major of the mode. `None` for chromatic material.
fn key_signature(key: Key) -> Option<(i8, bool)> {
    let tonic = key.tonic.semitone() as i32;
    let (major_root, minor) = match key.mode {
        Mode::Major | Mode::MajorPentatonic => (tonic, false),
        Mode::Minor | Mode::HarmonicMinor | Mode::MelodicMinor | Mode::MinorPentatonic => {
            (tonic + 3, true)
        }
        Mode::Dorian => (tonic - 2, false),
        Mode::Phrygian => (tonic - 4, false),
        Mode::Lydian => (tonic - 5, false),
        Mode::Mixolydian => (tonic - 7, false),
        Mode::Locrian => (tonic + 1, false),
        Mode::Chromatic => return None,
    };
    let fifths = (major_root.rem_euclid(12) * 7) % 12;
    let sharps = if fifths > 6 { fifths - 12 } else { fifths };
    Some((sharps as i8, minor))
}

#[cfg(test)]
mod tests {
    use super::*;
    use praxis_score::PitchClass;

    fn ks(tonic: PitchClass, mode: Mode) -> Option<(i8, bool)> {
        key_signature(Key { tonic, mode })
    }

    #[test]
    fn key_signatures() {
        assert_eq!(ks(PitchClass::C, Mode::Major), Some((0, false)));
        assert_eq!(ks(PitchClass::A, Mode::Minor), Some((0, true)));
        assert_eq!(ks(PitchClass::E, Mode::Minor), Some((1, true)));
        assert_eq!(ks(PitchClass::D, Mode::Minor), Some((-1, true)));
        assert_eq!(ks(PitchClass::Bb, Mode::Major), Some((-2, false)));
        assert_eq!(ks(PitchClass::Fs, Mode::Major), Some((6, false)));
        assert_eq!(ks(PitchClass::D, Mode::Dorian), Some((0, false)));
        assert_eq!(ks(PitchClass::G, Mode::Mixolydian), Some((0, false)));
        assert_eq!(ks(PitchClass::F, Mode::Lydian), Some((0, false)));
        assert_eq!(ks(PitchClass::E, Mode::Phrygian), Some((0, false)));
        assert_eq!(ks(PitchClass::B, Mode::Locrian), Some((0, false)));
        assert_eq!(ks(PitchClass::C, Mode::Chromatic), None);
    }

    #[test]
    fn tempo_and_meter_meta() {
        assert_eq!(micros_per_quarter(128.0), 468_750);
        assert_eq!(micros_per_quarter(120.0), 500_000);
        assert_eq!(
            time_signature(6, 8),
            MetaMessage::TimeSignature(6, 3, 12, 8)
        );
        assert_eq!(
            time_signature(4, 4),
            MetaMessage::TimeSignature(4, 2, 24, 8)
        );
    }
}
