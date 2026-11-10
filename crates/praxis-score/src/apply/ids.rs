//! Deterministic ids for patterns created by patches (rule 7).

use crate::types::{BarRange, Pattern, Score};
use crate::validate::valid_id;

/// Maximum id length accepted by the validator.
const MAX_ID: usize = 64;

/// `stem + tail`, plus `_<n>` for `n >= 2`, with the stem shortened so the id fits 64 chars.
fn compose(stem: &str, tail: &str, n: u32) -> String {
    let suffix = if n >= 2 {
        format!("_{n}")
    } else {
        String::new()
    };
    let room = MAX_ID.saturating_sub(tail.len() + suffix.len()).max(1);
    let stem: String = stem.chars().take(room).collect();
    format!("{stem}{tail}{suffix}")
}

/// True when two patterns are identical apart from their id.
fn same_content(a: &Pattern, b: &Pattern) -> bool {
    a.grid == b.grid
        && a.length_steps == b.length_steps
        && a.notes == b.notes
        && a.spoken == b.spoken
}

/// Where the pattern of a `write_bars` op lives: its id, and whether it must be added.
///
/// - its own id when that id is valid and free;
/// - an existing pattern that is identical (same id, or the derived id) is reused, never copied;
/// - otherwise `<layer>_b<start>-<end>`, then `_2`, `_3`, ... until the id is free or holds an
///   identical pattern. An existing different pattern is never replaced (copy-on-write).
pub(super) fn resolve_write_pattern(
    s: &Score,
    layer: &str,
    bars: BarRange,
    pattern: &Pattern,
) -> Result<(String, bool), String> {
    if valid_id(&pattern.id) {
        match s.pattern(&pattern.id) {
            None => return Ok((pattern.id.clone(), true)),
            Some(p) if p == pattern => return Ok((pattern.id.clone(), false)),
            Some(_) => {}
        }
    }
    let tail = format!("_b{}-{}", bars.start, bars.end);
    for n in 1..=10_000u32 {
        let id = compose(layer, &tail, n);
        match s.pattern(&id) {
            None => return Ok((id, true)),
            Some(p) if same_content(p, pattern) => return Ok((id, false)),
            Some(_) => {}
        }
    }
    Err(format!("no free pattern id for layer `{layer}` {tail}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_fits_and_suffixes() {
        assert_eq!(compose("piano", "_b13-16", 1), "piano_b13-16");
        assert_eq!(compose("piano", "_b13-16", 2), "piano_b13-16_2");
        let long = "a".repeat(64);
        let id = compose(&long, "_b13-16", 12);
        assert_eq!(id.len(), 64);
        assert!(valid_id(&id));
        assert!(id.ends_with("_b13-16_12"));
    }
}
