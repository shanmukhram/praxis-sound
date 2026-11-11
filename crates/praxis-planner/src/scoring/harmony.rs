//! Harmony for scored plans: progressions (chord symbols or Roman numerals), chords laid over
//! the form so each section starts its progression afresh, and voice-leading for chord layers.

use crate::plan::{PlanChord, ScorePlan, diatonic_symbol, parse_chord, scale_degree};
use praxis_score::{BarRange, Key, Mode};

/// Reads a Roman numeral ("i", "VI", "iv7", "vii°", "bVII") as a scale degree of the key, with
/// an optional seventh. Accidentals ("bVII", "#iv") mark a borrowed chord: `Err` with a message,
/// because Score v1 keeps every pitched note in the key.
pub fn roman_degree(token: &str) -> Option<Result<(i32, bool), String>> {
    let t = token.trim();
    let (acc, rest) = match t.chars().next() {
        Some(c @ ('b' | '#' | '♭' | '♯')) if t.len() > 1 => (Some(c), &t[c.len_utf8()..]),
        _ => (None, t),
    };
    let upper = rest.to_ascii_uppercase();
    const NUMERALS: [&str; 7] = ["VII", "VI", "IV", "V", "III", "II", "I"];
    let (numeral, suffix) = NUMERALS
        .iter()
        .find(|n| upper.starts_with(**n))
        .map(|n| (*n, &rest[n.len()..]))?;
    let suffix = suffix.trim();
    let seventh = suffix.contains('7');
    let ok_suffix = suffix.is_empty()
        || matches!(
            suffix,
            "7" | "°" | "o" | "dim" | "°7" | "o7" | "dim7" | "ø" | "ø7" | "+" | "maj7" | "M7"
        );
    if !ok_suffix {
        return None;
    }
    let degree = match numeral {
        "I" => 0,
        "II" => 1,
        "III" => 2,
        "IV" => 3,
        "V" => 4,
        "VI" => 5,
        _ => 6,
    };
    if let Some(a) = acc {
        return Some(Err(format!(
            "`{token}` ({a}{numeral}) is a borrowed chord outside the key; Score v1 keeps every \
             note in the key, so use a chord of the key"
        )));
    }
    Some(Ok((degree, seventh)))
}

/// One progression entry as a chord symbol in the key, or a message saying why not.
pub fn progression_symbol(key: &Key, token: &str) -> Result<String, String> {
    let t = token.trim();
    if t.chars().next().is_some_and(|c| ('A'..='G').contains(&c))
        && let Some(sym) = parse_chord(t)
    {
        if key.mode != Mode::Chromatic {
            for pc in [Some(sym.root), sym.bass].into_iter().flatten() {
                if scale_degree(key, pc).is_none() {
                    return Err(format!(
                        "chord `{t}` uses {} which is not in {}",
                        pc.name(),
                        key.display()
                    ));
                }
            }
        }
        return Ok(t.to_string());
    }
    match roman_degree(t) {
        Some(Ok((d, seventh))) => {
            let base = diatonic_symbol(key, d);
            if seventh && !base.ends_with("dim") && !base.ends_with("aug") {
                Ok(format!("{base}7"))
            } else {
                Ok(base)
            }
        }
        Some(Err(m)) => Err(m),
        None => Err(format!(
            "cannot read `{t}`: give chord symbols (\"Am\", \"F\") or Roman numerals (\"i\", \"VI\")"
        )),
    }
}

/// The progression's chord symbols, skipping entries that cannot be used.
pub fn progression_symbols(plan: &ScorePlan) -> Vec<String> {
    plan.progression
        .as_ref()
        .map(|p| {
            p.chords
                .iter()
                .filter_map(|c| progression_symbol(&plan.key, c).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Chords for every bar: the progression restarts at each section start, `bars_per_chord` bars
/// per chord, and a one-shot ends on the tonic chord.
pub fn chords_over_sections(
    plan: &ScorePlan,
    symbols: &[String],
    bars_per_chord: u32,
    one_shot: bool,
    total: u32,
) -> Vec<PlanChord> {
    if symbols.is_empty() {
        return vec![];
    }
    let bpc = bars_per_chord.clamp(1, 8);
    let mut per_bar: Vec<String> = Vec::new();
    let mut sections: Vec<BarRange> = plan.sections.iter().map(|s| s.bars).collect();
    sections.sort_by_key(|b| b.start);
    sections.retain(|b| b.start >= 1 && b.start <= b.end && b.end <= total);
    if sections.is_empty() {
        sections.push(BarRange {
            start: 1,
            end: total,
        });
    }
    for s in &sections {
        for i in 0..=(s.end.saturating_sub(s.start)) {
            per_bar.push(symbols[((i / bpc) as usize) % symbols.len()].clone());
        }
    }
    if one_shot && let Some(last) = per_bar.last_mut() {
        *last = diatonic_symbol(&plan.key, 0);
    }
    let mut out: Vec<PlanChord> = Vec::new();
    for (i, sym) in per_bar.into_iter().enumerate() {
        let bar = i as u32 + 1;
        match out.last_mut() {
            Some(c) if c.symbol == sym && c.bars.end + 1 == bar => c.bars.end = bar,
            _ => out.push(PlanChord {
                bars: BarRange {
                    start: bar,
                    end: bar,
                },
                symbol: sym,
            }),
        }
    }
    out
}

/// A library progression for an emotion preset, chosen by seed, as symbols in the key.
pub fn library_symbols(
    key: &Key,
    preset: &super::emotion::EmotionPreset,
    seed: u64,
) -> Vec<String> {
    let progs = preset.progressions;
    let prog = progs[(seed % progs.len() as u64) as usize];
    prog.iter().map(|d| diatonic_symbol(key, *d)).collect()
}

/// A close-position voicing of the pitch classes `pcs` (semitones 0..12) near `center`,
/// moving as little as possible from `prev` (total semitone motion of the sorted voices; the
/// first chord sits closest to the centre). Every voice stays within `lo..=hi`.
pub fn voice_lead(pcs: &[i32], prev: Option<&[i32]>, center: i32, lo: i32, hi: i32) -> Vec<i32> {
    let n = pcs.len();
    if n == 0 {
        return vec![];
    }
    let mut best: Option<(i64, Vec<i32>)> = None;
    // Each inversion, with its bass in each octave around the centre.
    for inv in 0..n {
        for base_oct in -2..=2 {
            let mut v: Vec<i32> = Vec::with_capacity(n);
            let first = pcs[inv].rem_euclid(12);
            let mut bass = (center / 12) * 12 + first + 12 * base_oct;
            while bass > center + 6 {
                bass -= 12;
            }
            while bass < center - 18 {
                bass += 12;
            }
            v.push(bass);
            for k in 1..n {
                let pc = pcs[(inv + k) % n].rem_euclid(12);
                let mut p = v[k - 1] + 1;
                while p.rem_euclid(12) != pc {
                    p += 1;
                }
                v.push(p);
            }
            let (Some(low), Some(high)) = (v.first(), v.last()) else {
                continue;
            };
            if *low < lo || *high > hi || high - low > 12 {
                continue;
            }
            let cost: i64 = match prev {
                Some(p) if p.len() == v.len() => {
                    v.iter()
                        .zip(p)
                        .map(|(a, b)| (a - b).abs() as i64)
                        .sum::<i64>()
                        * 4
                        + ((v.iter().sum::<i32>() / n as i32) - center).abs() as i64
                }
                _ => {
                    ((v.iter().sum::<i32>() / n as i32) - center).abs() as i64 * 4
                        + v[0] as i64 / 128
                }
            };
            if best
                .as_ref()
                .is_none_or(|(c, bv)| cost < *c || (cost == *c && v < *bv))
            {
                best = Some((cost, v));
            }
        }
    }
    best.map(|(_, v)| v).unwrap_or_else(|| {
        // No voicing fits the range: root position at the centre.
        let mut v = Vec::with_capacity(n);
        let mut p = (center / 12) * 12 + pcs[0].rem_euclid(12);
        v.push(p);
        for pc in &pcs[1..] {
            p += 1;
            while p.rem_euclid(12) != pc.rem_euclid(12) {
                p += 1;
            }
            v.push(p);
        }
        v
    })
}

/// Pitch classes (semitones above C) of the triad on scale degree `d`.
pub fn triad_pcs(key: &Key, d: i32) -> Vec<i32> {
    [d, d + 2, d + 4]
        .iter()
        .map(|x| key.degree_to_midi(*x, 4).rem_euclid(12))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use praxis_score::PitchClass;

    fn a_minor() -> Key {
        Key {
            tonic: PitchClass::A,
            mode: Mode::Minor,
        }
    }

    #[test]
    fn romans_and_symbols() {
        let k = a_minor();
        assert_eq!(progression_symbol(&k, "i").as_deref(), Ok("Am"));
        assert_eq!(progression_symbol(&k, "VI").as_deref(), Ok("F"));
        assert_eq!(progression_symbol(&k, "III").as_deref(), Ok("C"));
        assert_eq!(progression_symbol(&k, "VII").as_deref(), Ok("G"));
        assert_eq!(progression_symbol(&k, "iv7").as_deref(), Ok("Dm7"));
        assert_eq!(progression_symbol(&k, "ii°").as_deref(), Ok("Bdim"));
        assert_eq!(progression_symbol(&k, "F").as_deref(), Ok("F"));
        assert!(
            progression_symbol(&k, "bVII")
                .unwrap_err()
                .contains("borrowed")
        );
        assert!(progression_symbol(&k, "F#").is_err());
        assert!(progression_symbol(&k, "xyz").is_err());
    }

    #[test]
    fn voice_leading_moves_little() {
        let k = a_minor();
        // i - VI - III - VII in A minor.
        let mut prev: Option<Vec<i32>> = None;
        let mut total = 0;
        for d in [0, 5, 2, 6, 0] {
            let v = voice_lead(&triad_pcs(&k, d), prev.as_deref(), 60, 48, 76);
            assert_eq!(v.len(), 3);
            assert!(v.windows(2).all(|w| w[0] < w[1]));
            assert!(v.iter().all(|p| k.contains(*p as u8)));
            if let Some(p) = &prev {
                let moved: i32 = v.iter().zip(p).map(|(a, b)| (a - b).abs()).sum();
                assert!(moved <= 5, "{p:?} -> {v:?}");
                total += moved;
            }
            prev = Some(v);
        }
        assert!(total <= 14, "{total}");
    }
}
