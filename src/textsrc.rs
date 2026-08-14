//! Resolving text arguments (inline, file, stdin) and backslash escapes.

use std::io::Read;
use std::path::Path;

use crate::cli::TextSource;
use crate::error::{AppError, ErrorKind, Result};

/// Read a UTF-8 text argument. `what` names the argument in error messages.
pub fn resolve(src: &TextSource, what: &str, escapes: bool) -> Result<String> {
    resolve_pair(
        &src.text,
        &src.text_file,
        &format!("{what} requires --text or --text-file"),
        escapes,
    )
}

/// Same, for the ad-hoc `--x` / `--x-file` pairs (`--find`, `--with`).
pub fn resolve_pair(
    inline: &Option<String>,
    file: &Option<std::path::PathBuf>,
    what: &str,
    escapes: bool,
) -> Result<String> {
    let raw = if let Some(t) = inline {
        t.clone()
    } else if let Some(p) = file {
        read_utf8_file(p)?
    } else {
        return Err(AppError::new(ErrorKind::Usage, what.to_string()));
    };
    if escapes {
        unescape(&raw)
    } else {
        Ok(raw)
    }
}

/// Read a UTF-8 file, or standard input when the path is `-`.
///
/// One convention for "read this from stdin" across every path argument beats a
/// parallel `--x-stdin` flag on each of them.
pub fn read_utf8_file(path: &Path) -> Result<String> {
    if path == Path::new("-") {
        return read_stdin();
    }
    let bytes = std::fs::read(path).map_err(|e| {
        AppError::new(
            if e.kind() == std::io::ErrorKind::NotFound {
                ErrorKind::NotFound
            } else {
                ErrorKind::Io
            },
            format!("{}: {e}", path.display()),
        )
    })?;
    String::from_utf8(bytes).map_err(|_| {
        AppError::new(
            ErrorKind::Encoding,
            format!(
                "{} is not valid UTF-8; input text must be UTF-8",
                path.display()
            ),
        )
    })
}

pub fn read_stdin() -> Result<String> {
    let mut buf = Vec::new();
    std::io::stdin().read_to_end(&mut buf)?;
    String::from_utf8(buf)
        .map_err(|_| AppError::new(ErrorKind::Encoding, "standard input is not valid UTF-8"))
}

/// Interpret C-style backslash escapes so that multi-line text survives shells
/// and JSON-free argument passing.
pub fn unescape(s: &str) -> Result<String> {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            None => {
                return Err(AppError::new(
                    ErrorKind::Usage,
                    "text ends with a lone backslash",
                ))
            }
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('0') => out.push('\0'),
            Some('\\') => out.push('\\'),
            Some('\'') => out.push('\''),
            Some('"') => out.push('"'),
            Some('x') => {
                let hex: String = (&mut chars).take(2).collect();
                let v = u8::from_str_radix(&hex, 16).map_err(|_| {
                    AppError::new(ErrorKind::Usage, format!("invalid \\x escape: \\x{hex}"))
                })?;
                out.push(v as char);
            }
            Some('u') => {
                let mut hex = String::new();
                match chars.next() {
                    Some('{') => {
                        for c in chars.by_ref() {
                            if c == '}' {
                                break;
                            }
                            hex.push(c);
                        }
                    }
                    Some(first) => {
                        hex.push(first);
                        hex.extend((&mut chars).take(3));
                    }
                    None => {
                        return Err(AppError::new(ErrorKind::Usage, "truncated \\u escape"));
                    }
                }
                let v = u32::from_str_radix(&hex, 16).map_err(|_| {
                    AppError::new(ErrorKind::Usage, format!("invalid \\u escape: \\u{hex}"))
                })?;
                let ch = char::from_u32(v).ok_or_else(|| {
                    AppError::new(
                        ErrorKind::Usage,
                        format!("\\u{hex} is not a Unicode scalar value"),
                    )
                })?;
                out.push(ch);
            }
            Some(other) => {
                return Err(AppError::new(
                    ErrorKind::Usage,
                    format!("unknown escape sequence \\{other}"),
                ))
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes() {
        assert_eq!(unescape(r"a\nb").unwrap(), "a\nb");
        assert_eq!(unescape(r"\t\\x").unwrap(), "\t\\x");
        assert_eq!(unescape(r"é").unwrap(), "é");
        assert_eq!(unescape(r"\u{1F600}").unwrap(), "\u{1F600}");
        assert_eq!(unescape(r"\xE9").unwrap(), "é");
        assert!(unescape(r"a\").is_err());
        assert!(unescape(r"\q").is_err());
    }
}
