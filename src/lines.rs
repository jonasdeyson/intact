//! Line indexing, line-range parsing and end-of-line handling.

use std::str::FromStr;

use clap::ValueEnum;

use crate::error::{AppError, ErrorKind, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eol {
    Lf,
    CrLf,
    Cr,
}

impl Eol {
    pub fn as_str(self) -> &'static str {
        match self {
            Eol::Lf => "\n",
            Eol::CrLf => "\r\n",
            Eol::Cr => "\r",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Eol::Lf => "lf",
            Eol::CrLf => "crlf",
            Eol::Cr => "cr",
        }
    }
}

/// How newlines inside text supplied on the command line are treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum EolMode {
    /// Rewrite inserted newlines to match the file's dominant line ending.
    Auto,
    Lf,
    Crlf,
    Cr,
    /// Insert the text exactly as given.
    Keep,
}

impl EolMode {
    pub fn resolve(self, file_eol: Eol) -> Option<Eol> {
        match self {
            EolMode::Auto => Some(file_eol),
            EolMode::Lf => Some(Eol::Lf),
            EolMode::Crlf => Some(Eol::CrLf),
            EolMode::Cr => Some(Eol::Cr),
            EolMode::Keep => None,
        }
    }
}

/// Count line terminators and pick the dominant style. Ties and empty files
/// resolve to LF.
pub fn detect_eol(text: &str) -> (Eol, usize, usize, usize) {
    let b = text.as_bytes();
    let (mut lf, mut crlf, mut cr) = (0usize, 0usize, 0usize);
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\n' => {
                lf += 1;
                i += 1;
            }
            b'\r' => {
                if i + 1 < b.len() && b[i + 1] == b'\n' {
                    crlf += 1;
                    i += 2;
                } else {
                    cr += 1;
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    let dominant = if crlf > lf && crlf >= cr {
        Eol::CrLf
    } else if cr > lf && cr > crlf {
        Eol::Cr
    } else {
        Eol::Lf
    };
    (dominant, lf, crlf, cr)
}

/// Rewrite every line terminator in `text` to `eol`.
pub fn normalize_eol(text: &str, eol: Eol) -> String {
    let target = eol.as_str();
    let mut out = String::with_capacity(text.len());
    let b = text.as_bytes();
    let mut i = 0;
    let mut chunk_start = 0;
    while i < b.len() {
        match b[i] {
            b'\r' => {
                out.push_str(&text[chunk_start..i]);
                out.push_str(target);
                i += if i + 1 < b.len() && b[i + 1] == b'\n' {
                    2
                } else {
                    1
                };
                chunk_start = i;
            }
            b'\n' => {
                out.push_str(&text[chunk_start..i]);
                out.push_str(target);
                i += 1;
                chunk_start = i;
            }
            _ => i += 1,
        }
    }
    out.push_str(&text[chunk_start..]);
    out
}

pub fn ends_with_eol(text: &str) -> bool {
    text.ends_with('\n') || text.ends_with('\r')
}

#[derive(Debug, Clone, Copy)]
pub struct Line {
    /// Offset of the first byte of the line.
    pub start: usize,
    /// Offset just past the last byte of line content (before the terminator).
    pub content_end: usize,
    /// Offset just past the line terminator (== `content_end` on a final line
    /// without a terminator).
    pub end: usize,
}

#[derive(Debug, Default)]
pub struct LineIndex {
    pub lines: Vec<Line>,
}

impl LineIndex {
    pub fn build(text: &str) -> Self {
        let b = text.as_bytes();
        let mut lines = Vec::new();
        let mut start = 0usize;
        let mut i = 0usize;
        while i < b.len() {
            match b[i] {
                b'\n' => {
                    lines.push(Line {
                        start,
                        content_end: i,
                        end: i + 1,
                    });
                    i += 1;
                    start = i;
                }
                b'\r' => {
                    let end = if i + 1 < b.len() && b[i + 1] == b'\n' {
                        i + 2
                    } else {
                        i + 1
                    };
                    lines.push(Line {
                        start,
                        content_end: i,
                        end,
                    });
                    i = end;
                    start = i;
                }
                _ => i += 1,
            }
        }
        if start < b.len() {
            lines.push(Line {
                start,
                content_end: b.len(),
                end: b.len(),
            });
        }
        LineIndex { lines }
    }

    pub fn count(&self) -> usize {
        self.lines.len()
    }

    /// 1-based line number containing `offset`.
    pub fn line_of_offset(&self, offset: usize) -> usize {
        if self.lines.is_empty() {
            return 1;
        }
        self.lines.partition_point(|l| l.start <= offset).max(1)
    }

    pub fn get(&self, one_based: usize) -> Option<&Line> {
        if one_based == 0 {
            None
        } else {
            self.lines.get(one_based - 1)
        }
    }
}

/// A single line position: `7`, `$` (last line), or `-2` (second from last).
#[derive(Debug, Clone, Copy)]
pub enum LineSpec {
    Num(usize),
    FromEnd(usize),
    Last,
}

impl FromStr for LineSpec {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let s = s.trim();
        if s == "$" || s.eq_ignore_ascii_case("end") || s.eq_ignore_ascii_case("last") {
            return Ok(LineSpec::Last);
        }
        if let Some(rest) = s.strip_prefix('-') {
            let n: usize = rest
                .parse()
                .map_err(|_| format!("invalid line number '{s}'"))?;
            if n == 0 {
                return Err("line offset from end must be >= 1".to_string());
            }
            return Ok(LineSpec::FromEnd(n));
        }
        let n: usize = s
            .parse()
            .map_err(|_| format!("invalid line number '{s}'"))?;
        if n == 0 {
            return Err("line numbers are 1-based; 0 is not a valid line".to_string());
        }
        Ok(LineSpec::Num(n))
    }
}

impl LineSpec {
    /// Resolve against a file of `total` lines. `max` is the largest value the
    /// caller accepts (`total`, or `total + 1` for insertion points).
    pub fn resolve(self, total: usize, max: usize) -> Result<usize> {
        let n = match self {
            LineSpec::Num(n) => n,
            LineSpec::Last => total.max(1),
            LineSpec::FromEnd(k) => {
                if k > total {
                    return Err(AppError::new(
                        ErrorKind::Range,
                        format!("-{k} is before the start of a {total}-line file"),
                    ));
                }
                total - k + 1
            }
        };
        if n > max || n == 0 {
            return Err(AppError::new(
                ErrorKind::Range,
                format!("line {n} is out of range (file has {total} line(s))"),
            ));
        }
        Ok(n)
    }
}

/// An inclusive 1-based line range: `5`, `5:9`, `5:`, `:9`, `3:$`, `-3:-1`.
#[derive(Debug, Clone, Copy)]
pub struct LineRange {
    pub start: Option<LineSpec>,
    pub end: Option<LineSpec>,
}

impl FromStr for LineRange {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            return Err("empty line range".to_string());
        }
        let sep = s.find(':').or_else(|| s.find(".."));
        match sep {
            None => {
                let spec: LineSpec = s.parse()?;
                Ok(LineRange {
                    start: Some(spec),
                    end: Some(spec),
                })
            }
            Some(idx) => {
                let sep_len = if s[idx..].starts_with("..") { 2 } else { 1 };
                let (lhs, rhs) = (&s[..idx], &s[idx + sep_len..]);
                let start = if lhs.trim().is_empty() {
                    None
                } else {
                    Some(lhs.parse()?)
                };
                let end = if rhs.trim().is_empty() {
                    None
                } else {
                    Some(rhs.parse()?)
                };
                Ok(LineRange { start, end })
            }
        }
    }
}

impl LineRange {
    /// Resolve to an inclusive 1-based `(start, end)` pair.
    pub fn resolve(self, total: usize) -> Result<(usize, usize)> {
        if total == 0 {
            return Err(AppError::new(ErrorKind::Range, "file has no lines"));
        }
        let start = match self.start {
            Some(spec) => spec.resolve(total, total)?,
            None => 1,
        };
        let end = match self.end {
            Some(spec) => spec.resolve(total, total)?,
            None => total,
        };
        if end < start {
            return Err(AppError::new(
                ErrorKind::Range,
                format!("line range {start}:{end} ends before it starts"),
            ));
        }
        Ok((start, end))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_counts_lines() {
        assert_eq!(LineIndex::build("").count(), 0);
        assert_eq!(LineIndex::build("a").count(), 1);
        assert_eq!(LineIndex::build("a\n").count(), 1);
        assert_eq!(LineIndex::build("a\nb").count(), 2);
        assert_eq!(LineIndex::build("a\r\nb\r\n").count(), 2);
        assert_eq!(LineIndex::build("a\rb\r").count(), 2);
    }

    #[test]
    fn crlf_line_bounds() {
        let idx = LineIndex::build("ab\r\ncd\r\n");
        let l0 = idx.get(1).unwrap();
        assert_eq!((l0.start, l0.content_end, l0.end), (0, 2, 4));
        let l1 = idx.get(2).unwrap();
        assert_eq!((l1.start, l1.content_end, l1.end), (4, 6, 8));
    }

    #[test]
    fn eol_detection_prefers_dominant() {
        assert_eq!(detect_eol("a\r\nb\r\nc\n").0, Eol::CrLf);
        assert_eq!(detect_eol("a\nb\n").0, Eol::Lf);
        assert_eq!(detect_eol("").0, Eol::Lf);
        assert_eq!(detect_eol("a\rb\rc\r").0, Eol::Cr);
    }

    #[test]
    fn normalize_rewrites_terminators() {
        assert_eq!(normalize_eol("a\nb\r\nc\rd", Eol::CrLf), "a\r\nb\r\nc\r\nd");
        assert_eq!(normalize_eol("a\r\nb", Eol::Lf), "a\nb");
    }

    #[test]
    fn range_parsing() {
        let r: LineRange = "3:7".parse().unwrap();
        assert_eq!(r.resolve(10).unwrap(), (3, 7));
        let r: LineRange = "3:".parse().unwrap();
        assert_eq!(r.resolve(10).unwrap(), (3, 10));
        let r: LineRange = ":4".parse().unwrap();
        assert_eq!(r.resolve(10).unwrap(), (1, 4));
        let r: LineRange = "$".parse().unwrap();
        assert_eq!(r.resolve(10).unwrap(), (10, 10));
        let r: LineRange = "-2:-1".parse().unwrap();
        assert_eq!(r.resolve(10).unwrap(), (9, 10));
        let r: LineRange = "5".parse().unwrap();
        assert_eq!(r.resolve(10).unwrap(), (5, 5));
        assert!("7:3".parse::<LineRange>().unwrap().resolve(10).is_err());
        assert!("0".parse::<LineRange>().is_err());
    }
}
