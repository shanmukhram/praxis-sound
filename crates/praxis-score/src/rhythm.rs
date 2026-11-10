//! Spoken rhythm (FR-5). Implemented by WP-P (planner). Signatures are a FROZEN CONTRACT.
//! "DON din DON don TIN" -> one step per syllable: DON strong, din soft, don normal, TIN bright,
//! "-" or "." a rest, "~" a tie that extends the previous note by one step.
//!
//! # Token table
//!
//! Syllables are matched by spelling. All capitals means the syllable is emphasised; a leading
//! capital alone ("Don", "Din", "Tin") is read like the lower-case form, because it usually only
//! marks the start of a sentence.
//!
//! | Token | Step | Base velocity |
//! | --- | --- | --- |
//! | `DON` | strong hit (`Accent::Strong`) | 0.80 |
//! | `don`, `Don` | normal hit (`Accent::Normal`) | 0.65 |
//! | `din`, `Din` | soft hit (`Accent::Soft`) | 0.55 |
//! | `DIN` | an emphasised soft syllable: a normal hit (`Accent::Normal`) | 0.60 |
//! | `TIN` | bright hit (`Accent::Bright`) | 0.70 |
//! | `tin`, `Tin` | a quieter bright hit (`Accent::Bright`) | 0.60 |
//! | `-`, `.` | rest | |
//! | `~` | tie: the previous hit lasts one step longer | |
//!
//! Tokens are separated by whitespace or `|` (bar lines are only visual). Rest and tie marks may
//! also be glued to syllables or to each other: `DON~~ din--` is `DON ~ ~ din - -`. Base
//! velocities are before the accent multiplier the engine applies (contracts/score.md).
//!
//! Errors carry the offending token and its position, counted in characters from the start of
//! the text (0-based). A tie with no hit before it (first step, or right after a rest) is an
//! error. Empty input gives an empty list.

use crate::types::Accent;

#[derive(Clone, Debug, PartialEq)]
pub enum SpokenStep {
    Hit { accent: Accent, velocity: f32 },
    Rest,
    Tie,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("cannot read spoken rhythm token `{token}` at position {position}")]
pub struct SpokenError {
    pub token: String,
    pub position: usize,
}

/// Base velocity of a strong (`DON`) hit.
pub const VELOCITY_STRONG: f32 = 0.80;
/// Base velocity of a normal (`don`) hit.
pub const VELOCITY_NORMAL: f32 = 0.65;
/// Base velocity of a soft (`din`) hit.
pub const VELOCITY_SOFT: f32 = 0.55;
/// Base velocity of a bright (`TIN`) hit.
pub const VELOCITY_BRIGHT: f32 = 0.70;

fn syllable(word: &str) -> Option<SpokenStep> {
    let hit = |accent, velocity| Some(SpokenStep::Hit { accent, velocity });
    match word {
        "DON" => hit(Accent::Strong, VELOCITY_STRONG),
        "don" | "Don" => hit(Accent::Normal, VELOCITY_NORMAL),
        "din" | "Din" => hit(Accent::Soft, VELOCITY_SOFT),
        "DIN" => hit(Accent::Normal, 0.60),
        "TIN" => hit(Accent::Bright, VELOCITY_BRIGHT),
        "tin" | "Tin" => hit(Accent::Bright, 0.60),
        _ => None,
    }
}

fn is_mark(c: char) -> bool {
    matches!(c, '-' | '.' | '~')
}

/// Splits the text into (token, char position) pairs: words and single rest/tie marks.
fn tokens(text: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut word_start = 0usize;
    for (pos, c) in text.chars().enumerate() {
        if c.is_whitespace() || c == '|' || is_mark(c) {
            if !word.is_empty() {
                out.push((std::mem::take(&mut word), word_start));
            }
            if is_mark(c) {
                out.push((c.to_string(), pos));
            }
        } else {
            if word.is_empty() {
                word_start = pos;
            }
            word.push(c);
        }
    }
    if !word.is_empty() {
        out.push((word, word_start));
    }
    out
}

pub fn parse_spoken(text: &str) -> Result<Vec<SpokenStep>, SpokenError> {
    let mut steps = Vec::new();
    // Whether a tie at this point has a hit to extend.
    let mut can_tie = false;
    for (token, position) in tokens(text) {
        let step = match token.as_str() {
            "-" | "." => {
                can_tie = false;
                SpokenStep::Rest
            }
            "~" => {
                if !can_tie {
                    return Err(SpokenError { token, position });
                }
                SpokenStep::Tie
            }
            word => match syllable(word) {
                Some(hit) => {
                    can_tie = true;
                    hit
                }
                None => return Err(SpokenError { token, position }),
            },
        };
        steps.push(step);
    }
    Ok(steps)
}

/// True when every whitespace-separated word of `text` is a spoken-rhythm token and there are at
/// least `min_hits` hits. Used to recognise a spoken rhythm inside a brief.
pub fn looks_spoken(text: &str, min_hits: usize) -> bool {
    match parse_spoken(text) {
        Ok(steps) => {
            steps
                .iter()
                .filter(|s| matches!(s, SpokenStep::Hit { .. }))
                .count()
                >= min_hits
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accents(text: &str) -> Vec<Option<Accent>> {
        parse_spoken(text)
            .unwrap()
            .into_iter()
            .map(|s| match s {
                SpokenStep::Hit { accent, .. } => Some(accent),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn prd_example() {
        let steps = parse_spoken("DON din DON don TIN").unwrap();
        assert_eq!(
            steps,
            vec![
                SpokenStep::Hit {
                    accent: Accent::Strong,
                    velocity: VELOCITY_STRONG
                },
                SpokenStep::Hit {
                    accent: Accent::Soft,
                    velocity: VELOCITY_SOFT
                },
                SpokenStep::Hit {
                    accent: Accent::Strong,
                    velocity: VELOCITY_STRONG
                },
                SpokenStep::Hit {
                    accent: Accent::Normal,
                    velocity: VELOCITY_NORMAL
                },
                SpokenStep::Hit {
                    accent: Accent::Bright,
                    velocity: VELOCITY_BRIGHT
                },
            ]
        );
    }

    #[test]
    fn rests_ties_and_separators() {
        let steps = parse_spoken("DON ~ - din | . TIN ~ ~").unwrap();
        assert_eq!(steps.len(), 8);
        assert_eq!(steps[1], SpokenStep::Tie);
        assert_eq!(steps[2], SpokenStep::Rest);
        assert_eq!(steps[4], SpokenStep::Rest);
        assert_eq!(steps[6], SpokenStep::Tie);
        assert_eq!(steps[7], SpokenStep::Tie);
    }

    #[test]
    fn glued_marks() {
        assert_eq!(
            parse_spoken("DON~~ din--").unwrap(),
            parse_spoken("DON ~ ~ din - -").unwrap()
        );
        assert_eq!(parse_spoken("--..").unwrap(), vec![SpokenStep::Rest; 4]);
    }

    #[test]
    fn case_variants() {
        assert_eq!(
            accents("Don DIN Din tin Tin"),
            vec![
                Some(Accent::Normal),
                Some(Accent::Normal),
                Some(Accent::Soft),
                Some(Accent::Bright),
                Some(Accent::Bright),
            ]
        );
        match &parse_spoken("tin").unwrap()[0] {
            SpokenStep::Hit { velocity, .. } => assert!(*velocity < VELOCITY_BRIGHT),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn errors_name_token_and_position() {
        let e = parse_spoken("DON din BOOM don").unwrap_err();
        assert_eq!(e.token, "BOOM");
        assert_eq!(e.position, 8);
        assert!(e.to_string().contains("BOOM"));
        // Mixed case that is not in the table.
        let e = parse_spoken("DON dOn").unwrap_err();
        assert_eq!((e.token.as_str(), e.position), ("dOn", 4));
    }

    #[test]
    fn tie_needs_a_hit() {
        let e = parse_spoken("~ DON").unwrap_err();
        assert_eq!((e.token.as_str(), e.position), ("~", 0));
        let e = parse_spoken("DON - ~").unwrap_err();
        assert_eq!((e.token.as_str(), e.position), ("~", 6));
        assert!(parse_spoken("DON ~ ~").is_ok());
    }

    #[test]
    fn empty_and_detection() {
        assert_eq!(parse_spoken("").unwrap(), vec![]);
        assert_eq!(parse_spoken("  |  ").unwrap(), vec![]);
        assert!(looks_spoken("DON din DON don TIN", 3));
        assert!(!looks_spoken("soft felt piano", 1));
        assert!(!looks_spoken("DON -", 2));
    }

    #[test]
    fn unicode_positions_count_chars() {
        let e = parse_spoken("DON é").unwrap_err();
        assert_eq!((e.token.as_str(), e.position), ("é", 4));
    }
}
