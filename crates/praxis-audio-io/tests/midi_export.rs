//! MIDI export of the acceptance-1 bell Score, re-parsed with midly.

mod common;

use common::*;
use midly::{Format, MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};
use praxis_audio_io::midi::score_to_midi;
use praxis_score::Score;
use praxis_score::timeline::expand_notes;

fn bell() -> Score {
    let text = std::fs::read_to_string(repo_fixture("acceptance1_bell.score.json")).unwrap();
    praxis_score::from_json(&text).unwrap()
}

struct Note {
    on: u64,
    off: u64,
    channel: u8,
    key: u8,
    vel: u8,
}

/// Absolute-time notes of one track, pairing each note-on with the next note-off of its key.
fn notes(track: &[midly::TrackEvent]) -> Vec<Note> {
    let mut t = 0u64;
    let mut open: Vec<Note> = Vec::new();
    let mut done = Vec::new();
    for e in track {
        t += e.delta.as_int() as u64;
        if let TrackEventKind::Midi { channel, message } = e.kind {
            match message {
                MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => open.push(Note {
                    on: t,
                    off: 0,
                    channel: channel.as_int(),
                    key: key.as_int(),
                    vel: vel.as_int(),
                }),
                MidiMessage::NoteOff { key, .. } | MidiMessage::NoteOn { key, .. } => {
                    let i = open
                        .iter()
                        .position(|n| n.key == key.as_int() && n.channel == channel.as_int())
                        .expect("note-off without note-on");
                    let mut n = open.remove(i);
                    n.off = t;
                    done.push(n);
                }
                _ => {}
            }
        }
    }
    assert!(open.is_empty(), "every note is closed");
    done.sort_by_key(|n| (n.on, n.key));
    done
}

fn track_name<'a>(track: &'a [midly::TrackEvent<'a>]) -> String {
    track
        .iter()
        .find_map(|e| match e.kind {
            TrackEventKind::Meta(MetaMessage::TrackName(n)) => {
                Some(String::from_utf8_lossy(n).into_owned())
            }
            _ => None,
        })
        .unwrap_or_default()
}

fn end_tick(track: &[midly::TrackEvent]) -> u64 {
    let mut t = 0;
    for e in track {
        t += e.delta.as_int() as u64;
    }
    assert!(matches!(
        track.last().unwrap().kind,
        TrackEventKind::Meta(MetaMessage::EndOfTrack)
    ));
    t
}

#[test]
fn acceptance1_bell_exports_a_valid_type1_file() {
    let score = bell();
    let bytes = score_to_midi(&score);
    let smf = Smf::parse(&bytes).expect("midly parses the export");
    assert_eq!(smf.header.format, Format::Parallel, "SMF type 1");
    assert_eq!(smf.header.timing, Timing::Metrical(480.into()));
    assert_eq!(smf.tracks.len(), 1 + score.layers.len());
    assert_eq!(smf.tracks.len(), 3);

    // Conductor track.
    let conductor = &smf.tracks[0];
    let metas: Vec<&MetaMessage> = conductor
        .iter()
        .filter_map(|e| match &e.kind {
            TrackEventKind::Meta(m) => Some(m),
            _ => None,
        })
        .collect();
    let tempo = metas
        .iter()
        .find_map(|m| match m {
            MetaMessage::Tempo(t) => Some(t.as_int()),
            _ => None,
        })
        .expect("tempo");
    assert_eq!(tempo, 468_750);
    assert!(
        (60_000_000.0 / tempo as f64 - 128.0).abs() < 1e-9,
        "128 BPM"
    );
    assert!(metas.contains(&&MetaMessage::TimeSignature(4, 2, 24, 8)));
    assert!(
        metas.contains(&&MetaMessage::KeySignature(0, true)),
        "A minor"
    );
    assert!(
        !conductor
            .iter()
            .any(|e| matches!(e.kind, TrackEventKind::Midi { .. }))
    );
    assert_eq!(end_tick(conductor), 16 * 1920);

    // Layer tracks.
    let piano = notes(&smf.tracks[1]);
    let kick = notes(&smf.tracks[2]);
    assert_eq!(track_name(&smf.tracks[1]), "Felt piano");
    assert_eq!(track_name(&smf.tracks[2]), "Soft kick");
    assert_eq!(piano.len(), 144);
    assert_eq!(kick.len(), 16);
    assert!(piano.iter().all(|n| n.channel == 0));
    assert!(kick.iter().all(|n| n.channel == 9), "drums on channel 10");
    assert!(
        piano
            .iter()
            .chain(&kick)
            .all(|n| (1..=127).contains(&n.vel))
    );

    // Every onset is on the eighth grid (240 ticks); the motif starts at tick 0 and the kick
    // enters at bar 9.
    assert!(piano.iter().chain(&kick).all(|n| n.on % 240 == 0));
    assert_eq!((piano[0].on, piano[0].key), (0, 45));
    assert_eq!((piano[1].on, piano[1].key), (0, 57));
    assert_eq!(piano[1].off, 384, "length 1.6 eighths = 384 ticks");
    assert_eq!(piano[0].off, 1920, "length 8 eighths = one bar");
    assert_eq!(piano[2].on, 240);
    assert_eq!(kick[0].on, 8 * 1920);
    assert_eq!(kick[1].on, 8 * 1920 + 4 * 240);
    assert_eq!(kick[0].off - kick[0].on, 240);

    // Velocities come from the expanded events (velocity x accent, clamped).
    assert_eq!(piano[1].vel, 127, "0.8 strong -> 1.0");
    let soft_e = piano.iter().find(|n| n.on == 240).unwrap();
    assert_eq!((soft_e.key, soft_e.vel), (64, 49), "0.55 soft -> 0.385");

    // Notes agree one-to-one with the engine's expansion.
    let expanded = expand_notes(&score, 48_000);
    for (li, track_notes) in [(0usize, &piano), (1, &kick)] {
        let mut want: Vec<(u64, u8)> = expanded
            .iter()
            .filter(|n| n.layer == li)
            .map(|n| ((n.start_s * 128.0 / 60.0 * 480.0).round() as u64, n.pitch))
            .collect();
        want.sort();
        let got: Vec<(u64, u8)> = track_notes.iter().map(|n| (n.on, n.key)).collect();
        assert_eq!(got, want);
    }
}

#[test]
fn muted_layers_are_exported_and_flagged() {
    let mut score = bell();
    score.layers[1].muted = true;
    let smf_bytes = score_to_midi(&score);
    let smf = Smf::parse(&smf_bytes).unwrap();
    assert_eq!(track_name(&smf.tracks[2]), "Soft kick (muted)");
    assert_eq!(notes(&smf.tracks[2]).len(), 16);
}

#[test]
fn export_is_deterministic_and_handles_odd_meters() {
    let score = bell();
    assert_eq!(score_to_midi(&score), score_to_midi(&score));

    let mut s = bell();
    s.time_signature.numerator = 7;
    s.time_signature.denominator = 8;
    s.tempo.bpm = 97.3;
    s.layers[0].name = String::new();
    let smf_bytes = score_to_midi(&s);
    let smf = Smf::parse(&smf_bytes).unwrap();
    assert_eq!(
        track_name(&smf.tracks[1]),
        "piano",
        "falls back to the layer id"
    );
    assert!(smf.tracks[0].iter().any(|e| matches!(
        e.kind,
        TrackEventKind::Meta(MetaMessage::TimeSignature(7, 3, 12, 8))
    )));
    // 7/8 bar = 3.5 quarters = 1680 ticks; the score is 16 bars.
    assert_eq!(end_tick(&smf.tracks[0]), 16 * 1680);
}

#[test]
fn empty_score_has_only_a_conductor_track() {
    let smf_bytes = score_to_midi(&Score::empty(4, 120.0));
    let smf = Smf::parse(&smf_bytes).unwrap();
    assert_eq!(smf.tracks.len(), 1);
    assert_eq!(end_tick(&smf.tracks[0]), 4 * 1920);
}
