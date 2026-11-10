//! Bar phrases ("last four bars", "second half", "bars 9-12") and their resolution against a
//! Score. The rule provider uses [`find_phrase`] to spot them in an instruction; the planner
//! uses [`resolve_phrase`] to turn any phrase a provider wrote into an exact bar range.

use praxis_score::BarRange;

/// A bar phrase, independent of the Score's length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phrase {
    /// The last `n` bars.
    Last(u32),
    /// The first `n` bars.
    First(u32),
    FirstHalf,
    SecondHalf,
    /// Bars `a` to `b` inclusive.
    Bars(u32, u32),
    /// From bar `a` to the end.
    From(u32),
    /// From bar 1 to bar `b`.
    Until(u32),
    Whole,
}

impl Phrase {
    /// The bar range this phrase names in a Score of `bars` bars.
    pub fn range(self, bars: u32) -> Result<BarRange, String> {
        let r = |start: u32, end: u32| BarRange { start, end };
        let range = match self {
            Phrase::Last(n) => r(bars.saturating_sub(n).saturating_add(1).max(1), bars),
            Phrase::First(n) => r(1, n.min(bars)),
            Phrase::FirstHalf => r(1, (bars / 2).max(1)),
            Phrase::SecondHalf => r((bars / 2).saturating_add(1).min(bars), bars),
            Phrase::Bars(a, b) => r(a.min(b), a.max(b)),
            Phrase::From(a) => r(a, bars),
            Phrase::Until(b) => r(1, b),
            Phrase::Whole => r(1, bars),
        };
        let n = match self {
            Phrase::Last(n) | Phrase::First(n) => n,
            _ => 1,
        };
        if n == 0 || range.start == 0 || range.end < range.start || range.end > bars {
            return Err(format!(
                "{} names bars {}-{}, but the score has bars 1-{bars}",
                self.describe(),
                range.start,
                range.end
            ));
        }
        if matches!(self, Phrase::Last(n) | Phrase::First(n) if n > bars) {
            return Err(format!(
                "{} is longer than the score's {bars} bars",
                self.describe()
            ));
        }
        Ok(range)
    }

    /// Canonical text, e.g. "last 4 bars".
    pub fn describe(self) -> String {
        match self {
            Phrase::Last(1) => "the last bar".into(),
            Phrase::Last(n) => format!("the last {n} bars"),
            Phrase::First(1) => "the first bar".into(),
            Phrase::First(n) => format!("the first {n} bars"),
            Phrase::FirstHalf => "the first half".into(),
            Phrase::SecondHalf => "the second half".into(),
            Phrase::Bars(a, b) if a == b => format!("bar {a}"),
            Phrase::Bars(a, b) => format!("bars {a}-{b}"),
            Phrase::From(a) => format!("from bar {a}"),
            Phrase::Until(b) => format!("until bar {b}"),
            Phrase::Whole => "the whole score".into(),
        }
    }
}

/// The phrase forms the planner understands, for issue messages.
pub const PHRASE_FORMS: &str = "\"last N bars\", \"first N bars\", \"first half\", \
\"second half\", \"bars A-B\", \"bar N\", \"from bar N\", \"until bar N\", \"whole\"";

/// Lower-case words and numbers of a text. Digits and letters split ("9-12" -> "9", "-",
/// "12"); every other character separates.
pub fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cur_digit = false;
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        if !cur.is_empty() {
            out.push(std::mem::take(cur));
        }
    };
    for c in text.chars() {
        let c = match c {
            '\u{2013}' | '\u{2014}' | '\u{2212}' => '-',
            '\u{2019}' => '\'',
            c => c,
        };
        if c.is_ascii_digit() {
            if !cur_digit {
                flush(&mut cur, &mut out);
            }
            cur_digit = true;
            cur.push(c);
        } else if c.is_alphanumeric() || c == '\'' {
            if cur_digit {
                flush(&mut cur, &mut out);
            }
            cur_digit = false;
            cur.extend(c.to_lowercase());
        } else {
            flush(&mut cur, &mut out);
            cur_digit = false;
            if c == '-' {
                out.push("-".into());
            }
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// A number written as digits or a word ("4", "four", "sixteen").
pub fn number(w: &str) -> Option<u32> {
    if let Ok(n) = w.parse::<u32>() {
        return Some(n);
    }
    const WORDS: [&str; 21] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
        "twenty",
    ];
    match w {
        "a" | "an" | "single" => Some(1),
        "couple" | "pair" => Some(2),
        "thirty-two" | "thirtytwo" => Some(32),
        _ => WORDS.iter().position(|x| *x == w).map(|i| i as u32),
    }
}

fn is_bar_word(w: &str) -> bool {
    matches!(w, "bar" | "bars" | "measure" | "measures")
}

fn is_range_word(w: &str) -> bool {
    matches!(w, "-" | "to" | "through" | "thru" | "till" | "until")
}

/// Skips "the" / "very" fillers starting at `i`.
fn skip_fillers(w: &[String], mut i: usize) -> usize {
    while i < w.len() && matches!(w[i].as_str(), "the" | "very" | "its" | "of") {
        i += 1;
    }
    i
}

/// The first bar phrase in `w` at or after `from`: `(phrase, first word index, end index)`.
pub fn find_phrase(w: &[String], from: usize) -> Option<(Phrase, usize, usize)> {
    let at = |i: usize| w.get(i).map(String::as_str).unwrap_or("");
    for i in from..w.len() {
        let word = at(i);
        // "last/final/closing N bars", "last bar", "first/opening N bars".
        let last = matches!(word, "last" | "final" | "closing" | "ending");
        let first = matches!(word, "first" | "opening");
        if last || first {
            let j = skip_fillers(w, i + 1);
            if is_bar_word(at(j)) {
                let p = if last {
                    Phrase::Last(1)
                } else {
                    Phrase::First(1)
                };
                return Some((p, i, j + 1));
            }
            if let Some(n) = number(at(j))
                && is_bar_word(at(j + 1))
            {
                let p = if last {
                    Phrase::Last(n)
                } else {
                    Phrase::First(n)
                };
                return Some((p, i, j + 2));
            }
            if at(j) == "half" {
                let p = if last {
                    Phrase::SecondHalf
                } else {
                    Phrase::FirstHalf
                };
                return Some((p, i, j + 1));
            }
        }
        if matches!(word, "second" | "back" | "latter") && at(skip_fillers(w, i + 1)) == "half" {
            return Some((Phrase::SecondHalf, i, skip_fillers(w, i + 1) + 1));
        }
        if matches!(word, "front" | "former") && at(skip_fillers(w, i + 1)) == "half" {
            return Some((Phrase::FirstHalf, i, skip_fillers(w, i + 1) + 1));
        }
        // "from bar N (on)".
        if matches!(word, "from" | "after" | "starting") {
            let mut j = i + 1;
            if at(j) == "at" {
                j += 1;
            }
            if is_bar_word(at(j))
                && let Some(n) = number(at(j + 1))
            {
                if is_range_word(at(j + 2))
                    && let Some(m) = number(at(j + 3))
                {
                    return Some((Phrase::Bars(n, m), i, j + 4));
                }
                let n = if word == "after" { n + 1 } else { n };
                return Some((Phrase::From(n), i, j + 2));
            }
        }
        // "until / up to / through bar N".
        if matches!(word, "until" | "till" | "through" | "to")
            && is_bar_word(at(i + 1))
            && let Some(n) = number(at(i + 2))
        {
            let start = if word == "to" && at(i.wrapping_sub(1)) != "up" {
                None
            } else {
                Some(i)
            };
            if let Some(s) = start {
                return Some((Phrase::Until(n), s, i + 3));
            }
        }
        // "bars A-B", "bars A to B", "bar N".
        if is_bar_word(word)
            && let Some(a) = number(at(i + 1))
        {
            if is_range_word(at(i + 2))
                && let Some(b) = number(at(i + 3))
            {
                return Some((Phrase::Bars(a, b), i, i + 4));
            }
            return Some((Phrase::Bars(a, a), i, i + 2));
        }
        if matches!(word, "whole" | "entire" | "everything" | "throughout") {
            return Some((Phrase::Whole, i, i + 1));
        }
    }
    None
}

/// Resolves a phrase a provider wrote to a bar range of a Score with `bars` bars.
pub fn resolve_phrase(text: &str, bars: u32) -> Result<BarRange, String> {
    let w = words(text);
    if w.is_empty()
        || w.iter()
            .all(|x| matches!(x.as_str(), "all" | "the" | "score"))
    {
        return Ok(BarRange {
            start: 1,
            end: bars,
        });
    }
    match find_phrase(&w, 0) {
        Some((p, _, _)) => p.range(bars),
        None => Err(format!(
            "cannot read the bar phrase \"{text}\"; use one of {PHRASE_FORMS}, or give `bars` \
             as {{start, end}} within 1-{bars}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(text: &str, bars: u32) -> Result<(u32, u32), String> {
        resolve_phrase(text, bars).map(|b| (b.start, b.end))
    }

    #[test]
    fn phrases_resolve() {
        assert_eq!(r("last four bars", 16), Ok((13, 16)));
        assert_eq!(r("the last 4 bars", 16), Ok((13, 16)));
        assert_eq!(r("final bar", 16), Ok((16, 16)));
        assert_eq!(r("first 8 bars", 16), Ok((1, 8)));
        assert_eq!(r("first half", 16), Ok((1, 8)));
        assert_eq!(r("the second half", 16), Ok((9, 16)));
        assert_eq!(r("last half", 15), Ok((8, 15)));
        assert_eq!(r("bars 9-12", 16), Ok((9, 12)));
        assert_eq!(r("bars 9 to 12", 16), Ok((9, 12)));
        assert_eq!(r("bars 9\u{2013}12", 16), Ok((9, 12)));
        assert_eq!(r("bar 5", 16), Ok((5, 5)));
        assert_eq!(r("from bar 9", 16), Ok((9, 16)));
        assert_eq!(r("up to bar 4", 16), Ok((1, 4)));
        assert_eq!(r("whole", 16), Ok((1, 16)));
        assert_eq!(r("", 16), Ok((1, 16)));
        assert!(r("bars 9-20", 16).is_err());
        assert!(r("last twenty bars", 16).is_err());
        assert!(r("somewhere nice", 16).is_err());
    }

    #[test]
    fn words_split_numbers() {
        assert_eq!(words("Bars 9-12, then"), ["bars", "9", "-", "12", "then"]);
        assert_eq!(number("four"), Some(4));
        assert_eq!(number("16"), Some(16));
    }
}
