//! SFZ v1 parser: headers, the core opcodes in docs/contracts/instruments.md, `#define`,
//! `#include` (relative to the including file), `default_path`, `note_offset`/`octave_offset`,
//! `//` and `/* */` comments, and note names.
//!
//! Inheritance follows SFZ: `<global>` -> `<master>` -> `<group>` -> `<region>`, later levels
//! overriding earlier ones. Unsupported opcodes and headers produce warnings (deduplicated) and are
//! ignored; malformed input is an [`SfzError`] naming the file and line.

use super::region::{Applied, Region, SampleRef};
use std::collections::BTreeSet;

/// A parse error with its location.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{file}:{line}: {msg}")]
pub struct SfzError {
    pub file: String,
    pub line: usize,
    pub msg: String,
}

/// A parsed SFZ instrument.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SfzInstrument {
    pub regions: Vec<Region>,
    /// Non-fatal issues ("file:line: ..."), one per unsupported opcode or header name.
    pub warnings: Vec<String>,
}

const MAX_INCLUDE_DEPTH: usize = 16;

#[derive(Clone, Debug)]
struct Line {
    file: String,
    line: usize,
    text: String,
}

#[derive(Clone, Debug)]
struct Op {
    name: String,
    value: String,
    file: String,
    line: usize,
    /// `note_offset + 12 * octave_offset` when the opcode was read.
    key_offset: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    Control,
    Global,
    Master,
    Group,
    Region,
    Ignored,
}

/// Parses an SFZ file. `read` returns the text of a pack-relative path; it is used for the
/// file itself and for every `#include`. Sample paths resolve relative to `path`'s directory.
pub fn parse_file(
    path: &str,
    read: &mut dyn FnMut(&str) -> Result<String, String>,
) -> Result<SfzInstrument, SfzError> {
    let path = path.replace('\\', "/");
    let text = read(&path).map_err(|msg| SfzError {
        file: path.clone(),
        line: 0,
        msg,
    })?;
    parse_text(&text, &path, read)
}

/// Parses SFZ text that claims to live at pack-relative `path` (for sample resolution and
/// `#include`).
pub fn parse_text(
    text: &str,
    path: &str,
    read: &mut dyn FnMut(&str) -> Result<String, String>,
) -> Result<SfzInstrument, SfzError> {
    let mut lines = Vec::new();
    let mut defines: Vec<(String, String)> = Vec::new();
    preprocess(text, path, read, &mut defines, &mut lines, 0)?;
    Parser::new(dir_of(path)).run(&lines)
}

/// Parses standalone SFZ text with no includes.
pub fn parse_str(text: &str, path: &str) -> Result<SfzInstrument, SfzError> {
    parse_text(text, path, &mut |p: &str| {
        Err(format!("cannot include `{p}`: no file loader"))
    })
}

fn dir_of(path: &str) -> String {
    match path.rfind('/') {
        Some(i) => path[..i].to_string(),
        None => String::new(),
    }
}

/// Joins `rel` onto `base` (both pack-relative, '/'-separated), resolving `.` and `..`.
/// Rejects absolute paths and anything that climbs out of the pack store.
pub fn join_path(base: &str, rel: &str) -> Result<String, String> {
    let rel = rel.replace('\\', "/");
    if rel.starts_with('/') || rel.get(1..2) == Some(":") {
        return Err(format!("absolute path `{rel}` is not allowed"));
    }
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    for p in rel.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(format!("path `{rel}` leaves the pack directory"));
                }
            }
            p => parts.push(p),
        }
    }
    if parts.is_empty() {
        return Err(format!("empty path `{rel}`"));
    }
    Ok(parts.join("/"))
}

/// Replaces `/* ... */` and `// ...` comments with spaces, keeping newlines so line numbers hold.
fn strip_comments(text: &str, file: &str) -> Result<String, SfzError> {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut line = 1;
    let mut seg_start = 0;
    while i < b.len() {
        if b[i] == b'\n' {
            line += 1;
            i += 1;
        } else if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            out.push_str(&text[seg_start..i]);
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            seg_start = i;
        } else if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            out.push_str(&text[seg_start..i]);
            let start_line = line;
            i += 2;
            loop {
                if i + 1 >= b.len() {
                    return Err(SfzError {
                        file: file.to_string(),
                        line: start_line,
                        msg: "unterminated /* comment".into(),
                    });
                }
                if b[i] == b'*' && b[i + 1] == b'/' {
                    i += 2;
                    break;
                }
                if b[i] == b'\n' {
                    out.push('\n');
                    line += 1;
                } else {
                    out.push(' ');
                }
                i += 1;
            }
            seg_start = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&text[seg_start..]);
    Ok(out)
}

fn substitute(text: &str, defines: &[(String, String)]) -> String {
    if defines.is_empty() || !text.contains('$') {
        return text.to_string();
    }
    let mut out = text.to_string();
    // Longest names first so `$VEL10` is not replaced as `$VEL1` + "0".
    let mut order: Vec<&(String, String)> = defines.iter().collect();
    order.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
    for (k, v) in order {
        out = out.replace(k.as_str(), v);
    }
    out
}

fn preprocess(
    text: &str,
    file: &str,
    read: &mut dyn FnMut(&str) -> Result<String, String>,
    defines: &mut Vec<(String, String)>,
    out: &mut Vec<Line>,
    depth: usize,
) -> Result<(), SfzError> {
    let err = |line: usize, msg: String| SfzError {
        file: file.to_string(),
        line,
        msg,
    };
    let clean = strip_comments(text, file)?;
    for (idx, raw) in clean.lines().enumerate() {
        let line_no = idx + 1;
        let t = raw.trim();
        if let Some(rest) = t.strip_prefix("#define") {
            let mut it = rest.split_whitespace();
            let (Some(name), Some(_)) = (it.next(), it.clone().next()) else {
                return Err(err(line_no, "#define needs `$NAME value`".into()));
            };
            if !name.starts_with('$') || name.len() < 2 {
                return Err(err(
                    line_no,
                    format!("#define name `{name}` must start with $"),
                ));
            }
            let value = rest.trim_start()[name.len()..].trim();
            let value = substitute(value, defines);
            defines.retain(|(k, _)| k != name);
            defines.push((name.to_string(), value));
        } else if let Some(rest) = t.strip_prefix("#include") {
            let inc = substitute(rest.trim(), defines);
            let inc = inc.trim();
            let name = inc
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .ok_or_else(|| err(line_no, "#include needs a quoted file name".into()))?;
            if depth >= MAX_INCLUDE_DEPTH {
                return Err(err(line_no, "#include nested too deeply (cycle?)".into()));
            }
            let inc_path = join_path(&dir_of(file), name).map_err(|m| err(line_no, m))?;
            let inc_text = read(&inc_path)
                .map_err(|m| err(line_no, format!("cannot include `{name}`: {m}")))?;
            preprocess(&inc_text, &inc_path, read, defines, out, depth + 1)?;
        } else if t.starts_with('#') {
            return Err(err(line_no, format!("unknown directive `{t}`")));
        } else if !t.is_empty() {
            out.push(Line {
                file: file.to_string(),
                line: line_no,
                text: substitute(t, defines),
            });
        }
    }
    Ok(())
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// True when `s` starts with `identifier=`.
fn starts_opcode(s: &[u8]) -> bool {
    let mut i = 0;
    while i < s.len() && is_ident(s[i]) {
        i += 1;
    }
    i > 0 && i < s.len() && s[i] == b'='
}

enum Token {
    Header(String),
    Opcode(String, String),
}

fn tokenize(l: &Line) -> Result<Vec<Token>, SfzError> {
    let err = |msg: String| SfzError {
        file: l.file.clone(),
        line: l.line,
        msg,
    };
    let s = l.text.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < s.len() {
        if s[i].is_ascii_whitespace() {
            i += 1;
        } else if s[i] == b'<' {
            let end = l.text[i..]
                .find('>')
                .map(|e| i + e)
                .ok_or_else(|| err("header is missing `>`".into()))?;
            toks.push(Token::Header(l.text[i + 1..end].trim().to_string()));
            i = end + 1;
        } else {
            let start = i;
            while i < s.len() && is_ident(s[i]) {
                i += 1;
            }
            if i == start || i >= s.len() || s[i] != b'=' {
                let word: String = l.text[start..]
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_string();
                return Err(err(format!("expected `opcode=value`, found `{word}`")));
            }
            let name = l.text[start..i].to_string();
            i += 1;
            let vstart = i;
            // The value runs to the next `<header>`, the next ` opcode=`, or the end of line
            // (sample paths may contain spaces).
            while i < s.len() {
                if s[i] == b'<' {
                    break;
                }
                if s[i].is_ascii_whitespace() {
                    let mut j = i;
                    while j < s.len() && s[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j >= s.len() || s[j] == b'<' || starts_opcode(&s[j..]) {
                        break;
                    }
                    i = j;
                } else {
                    i += 1;
                }
            }
            let value = l.text[vstart..i].trim().to_string();
            if value.is_empty() {
                return Err(err(format!("opcode `{name}` has no value")));
            }
            // Only sample paths may contain spaces.
            if name != "sample"
                && let Some(extra) = value.split_whitespace().nth(1)
            {
                return Err(err(format!("unexpected `{extra}` after `{name}=`")));
            }
            toks.push(Token::Opcode(name, value));
        }
    }
    Ok(toks)
}

struct Parser {
    base_dir: String,
    default_path: String,
    note_offset: i32,
    octave_offset: i32,
    section: Section,
    global: Vec<Op>,
    master: Vec<Op>,
    group: Vec<Op>,
    region: Option<(Vec<Op>, String, usize)>,
    out: SfzInstrument,
    warned: BTreeSet<String>,
}

impl Parser {
    fn new(base_dir: String) -> Parser {
        Parser {
            base_dir,
            default_path: String::new(),
            note_offset: 0,
            octave_offset: 0,
            section: Section::None,
            global: Vec::new(),
            master: Vec::new(),
            group: Vec::new(),
            region: None,
            out: SfzInstrument::default(),
            warned: BTreeSet::new(),
        }
    }

    fn warn_once(&mut self, key: &str, msg: String) {
        if self.warned.insert(key.to_string()) {
            self.out.warnings.push(msg);
        }
    }

    fn run(mut self, lines: &[Line]) -> Result<SfzInstrument, SfzError> {
        for l in lines {
            for tok in tokenize(l)? {
                match tok {
                    Token::Header(h) => self.header(&h, l)?,
                    Token::Opcode(n, v) => self.opcode(n, v, l)?,
                }
            }
        }
        self.flush_region()?;
        Ok(self.out)
    }

    fn header(&mut self, h: &str, l: &Line) -> Result<(), SfzError> {
        self.flush_region()?;
        self.section = match h {
            "region" => {
                self.region = Some((Vec::new(), l.file.clone(), l.line));
                Section::Region
            }
            "group" => {
                self.group.clear();
                Section::Group
            }
            "master" => {
                self.master.clear();
                self.group.clear();
                Section::Master
            }
            "global" => {
                self.global.clear();
                self.master.clear();
                self.group.clear();
                Section::Global
            }
            "control" => Section::Control,
            "curve" | "effect" | "midi" => {
                self.warn_once(
                    &format!("<{h}>"),
                    format!(
                        "{}:{}: <{h}> sections are not supported; ignored",
                        l.file, l.line
                    ),
                );
                Section::Ignored
            }
            other => {
                return Err(SfzError {
                    file: l.file.clone(),
                    line: l.line,
                    msg: format!("unknown header <{other}>"),
                });
            }
        };
        Ok(())
    }

    fn opcode(&mut self, name: String, value: String, l: &Line) -> Result<(), SfzError> {
        let err = |msg: String| SfzError {
            file: l.file.clone(),
            line: l.line,
            msg,
        };
        if self.section == Section::Control {
            match name.as_str() {
                "default_path" => {
                    self.default_path = value.replace('\\', "/");
                }
                "note_offset" => {
                    self.note_offset = value
                        .trim()
                        .parse::<i32>()
                        .map_err(|_| err(format!("bad note_offset `{value}`")))?
                        .clamp(-127, 127)
                }
                "octave_offset" => {
                    self.octave_offset = value
                        .trim()
                        .parse::<i32>()
                        .map_err(|_| err(format!("bad octave_offset `{value}`")))?
                        .clamp(-10, 10)
                }
                other => self.warn_once(
                    other,
                    format!(
                        "{}:{}: unsupported <control> opcode `{other}` ignored",
                        l.file, l.line
                    ),
                ),
            }
            return Ok(());
        }
        let value = if name == "sample" {
            if value.starts_with('*') {
                return Err(err(format!(
                    "generated sample `{value}` is not supported (use a sample file)"
                )));
            }
            let dp = join_path(&self.base_dir, &self.default_path)
                .unwrap_or_else(|_| self.base_dir.clone());
            join_path(&dp, &value).map_err(err)?
        } else {
            value
        };
        let op = Op {
            name,
            value,
            file: l.file.clone(),
            line: l.line,
            key_offset: self.note_offset + 12 * self.octave_offset,
        };
        match self.section {
            Section::Global => self.global.push(op),
            Section::Master => self.master.push(op),
            Section::Group => self.group.push(op),
            Section::Region => {
                if let Some((ops, _, _)) = self.region.as_mut() {
                    ops.push(op);
                }
            }
            Section::Ignored => {}
            Section::None => return Err(err("opcode before any header".into())),
            Section::Control => {}
        }
        Ok(())
    }

    fn flush_region(&mut self) -> Result<(), SfzError> {
        let Some((ops, file, line)) = self.region.take() else {
            return Ok(());
        };
        let mut r = Region::new(SampleRef::File(String::new()));
        r.origin = format!("{file}:{line}");
        let all: Vec<Op> = self
            .global
            .iter()
            .chain(self.master.iter())
            .chain(self.group.iter())
            .chain(ops.iter())
            .cloned()
            .collect();
        for op in &all {
            let applied = r
                .apply_opcode(&op.name, &op.value, op.key_offset)
                .map_err(|msg| SfzError {
                    file: op.file.clone(),
                    line: op.line,
                    msg: format!("{}: {msg}", op.name),
                })?;
            if applied == Applied::Unsupported {
                self.warn_once(
                    &op.name,
                    format!(
                        "{}:{}: unsupported opcode `{}={}` ignored",
                        op.file, op.line, op.name, op.value
                    ),
                );
            }
        }
        if r.sample == SampleRef::File(String::new()) {
            return Err(SfzError {
                file,
                line,
                msg: "region has no sample".into(),
            });
        }
        r.validate().map_err(|msg| SfzError {
            file: file.clone(),
            line,
            msg,
        })?;
        self.out.regions.push(r);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(
            join_path("pack", "samples/a.wav").unwrap(),
            "pack/samples/a.wav"
        );
        assert_eq!(join_path("pack/sfz", "../s/a.wav").unwrap(), "pack/s/a.wav");
        assert_eq!(join_path("pack", "s\\a.wav").unwrap(), "pack/s/a.wav");
        assert!(join_path("pack", "../../etc/passwd").is_err());
        assert!(join_path("pack", "/etc/passwd").is_err());
        assert!(join_path("pack", "C:/x.wav").is_err());
    }

    #[test]
    fn value_with_spaces_and_trailing_opcode() {
        let i = parse_str(
            "<region> sample=My Piano/C 4.wav lokey=c4 hikey=e4",
            "p/x.sfz",
        )
        .unwrap();
        assert_eq!(
            i.regions[0].sample,
            SampleRef::File("p/My Piano/C 4.wav".into())
        );
        assert_eq!((i.regions[0].lokey, i.regions[0].hikey), (60, 64));
    }
}
