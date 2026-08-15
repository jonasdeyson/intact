//! Loading, editing and saving a file without disturbing its encoding.
//!
//! The central guarantee: when the file round-trips (decode → re-encode gives
//! back the original bytes exactly), edits are applied by *splicing* encoded
//! bytes into the original buffer. Every byte outside the edited region is the
//! byte that was already there, so repeated edits cannot accumulate mojibake.
//! Only when the file does not round-trip does the tool fall back to
//! re-encoding the whole file, and that path requires an explicit `--lossy`.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use encoding_rs::{Encoding, UTF_8};

use crate::encoding_util::{
    BomKind, EncodedMap, MojibakeHint, UnmappablePolicy, build_encoded_map, encode_text,
    is_stateful, scan_mojibake, sniff_bom,
};
use crate::error::{AppError, ErrorKind, Result};
use crate::lines::{Eol, LineIndex, detect_eol};

/// How the file's encoding was determined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detection {
    /// The user passed `--encoding`.
    Explicit,
    /// A byte-order mark was present.
    Bom,
    /// The bytes are valid UTF-8.
    Utf8,
    /// Statistically guessed (chardetng).
    Guessed,
    /// The file is empty or new.
    Default,
}

impl Detection {
    pub fn as_str(self) -> &'static str {
        match self {
            Detection::Explicit => "explicit",
            Detection::Bom => "bom",
            Detection::Utf8 => "utf-8-valid",
            Detection::Guessed => "guessed",
            Detection::Default => "default",
        }
    }
}

/// An encoding the caller imposed, and where it came from. Tracking the source
/// separately lets `info` say whether the encoding was a decision or a guess.
#[derive(Debug, Clone, Copy)]
pub struct ForcedEncoding {
    pub encoding: &'static Encoding,
    pub source: Detection,
}

impl ForcedEncoding {
    pub fn flag(encoding: &'static Encoding) -> Self {
        ForcedEncoding {
            encoding,
            source: Detection::Explicit,
        }
    }
}

/// A replacement of `text[start..end]` (UTF-8 offsets into the decoded text).
#[derive(Debug, Clone)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

impl Edit {
    pub fn new(start: usize, end: usize, text: impl Into<String>) -> Self {
        Edit {
            start,
            end,
            text: text.into(),
        }
    }
}

pub struct Document {
    pub path: PathBuf,
    /// Whole original file, BOM included.
    pub raw: Vec<u8>,
    pub bom: Option<BomKind>,
    pub encoding: &'static Encoding,
    pub detection: Detection,
    /// Decoded content, BOM excluded.
    pub text: String,
    pub eol: Eol,
    pub existed: bool,
    /// Decoding hit malformed byte sequences.
    pub had_decode_errors: bool,
    /// Re-encoding the decoded text reproduces the original bytes exactly.
    pub roundtrip: bool,
    map: Option<EncodedMap>,
    line_index: LineIndex,
}

impl Document {
    /// Read a file from disk. `encoding` overrides detection.
    pub fn load(path: &Path, encoding: Option<ForcedEncoding>) -> Result<Document> {
        let (raw, existed) = match fs::read(path) {
            Ok(bytes) => (bytes, true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(AppError::new(
                    ErrorKind::NotFound,
                    format!("no such file: {}", path.display()),
                )
                .with_hint("use `intact create` to make a new file"));
            }
            Err(e) => return Err(AppError::from(e)),
        };
        Ok(Document::from_bytes(
            path.to_path_buf(),
            raw,
            encoding,
            existed,
        ))
    }

    /// Load a file that is allowed to be missing (for `write` / `create`).
    pub fn load_or_empty(path: &Path, encoding: Option<ForcedEncoding>) -> Result<Document> {
        match fs::read(path) {
            Ok(bytes) => Ok(Document::from_bytes(
                path.to_path_buf(),
                bytes,
                encoding,
                true,
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Document::from_bytes(
                path.to_path_buf(),
                Vec::new(),
                encoding,
                false,
            )),
            Err(e) => Err(AppError::from(e)),
        }
    }

    pub fn from_bytes(
        path: PathBuf,
        raw: Vec<u8>,
        forced: Option<ForcedEncoding>,
        existed: bool,
    ) -> Document {
        let bom = sniff_bom(&raw);

        let (encoding, detection, bom) = match forced {
            Some(ForcedEncoding {
                encoding: enc,
                source,
            }) => {
                // Only strip a BOM that belongs to the forced encoding.
                let bom = bom.filter(|b| b.encoding() == enc);
                (enc, source, bom)
            }
            None => match bom {
                Some(b) => (b.encoding(), Detection::Bom, Some(b)),
                None => {
                    let body = &raw[..];
                    if body.is_empty() {
                        (UTF_8, Detection::Default, None)
                    } else if std::str::from_utf8(body).is_ok() {
                        (UTF_8, Detection::Utf8, None)
                    } else {
                        (
                            crate::encoding_util::detect_legacy(body),
                            Detection::Guessed,
                            None,
                        )
                    }
                }
            },
        };

        let content_start = bom.map(|b| b.len()).unwrap_or(0);
        let content = &raw[content_start.min(raw.len())..];
        // The BOM (if any) was already split off above, so BOM handling must be
        // disabled here or a second BOM-looking sequence would be swallowed.
        let (decoded, had_decode_errors) = {
            let (cow, had_errors) = encoding.decode_without_bom_handling(content);
            (cow.into_owned(), had_errors)
        };

        // Splicing needs a stable character→byte map, which stateful encoders
        // do not have.
        let map = if is_stateful(encoding) {
            None
        } else {
            build_encoded_map(encoding, &decoded)
        };
        let roundtrip = match &map {
            Some(m) => !had_decode_errors && m.encoded == content,
            None => false,
        };

        let (eol, _, _, _) = detect_eol(&decoded);
        let line_index = LineIndex::build(&decoded);

        Document {
            path,
            raw,
            bom,
            encoding,
            detection,
            text: decoded,
            eol,
            existed,
            had_decode_errors,
            roundtrip,
            map,
            line_index,
        }
    }

    pub fn lines(&self) -> &LineIndex {
        &self.line_index
    }

    pub fn content_start(&self) -> usize {
        self.bom.map(|b| b.len()).unwrap_or(0)
    }

    /// True if the raw bytes contain a NUL, which usually means the file is not
    /// text at all (UTF-16 excepted, where NULs are normal).
    pub fn looks_binary(&self) -> bool {
        if self.encoding == encoding_rs::UTF_16LE || self.encoding == encoding_rs::UTF_16BE {
            return false;
        }
        self.raw.contains(&0)
    }

    /// Mojibake-shaped sequences in the decoded text, if any. Computed on
    /// demand: only `info` and the write path ask, so the read-only commands
    /// should not pay for another pass over the text.
    pub fn mojibake(&self) -> Option<MojibakeHint> {
        scan_mojibake(&self.text)
    }

    /// A warning about mojibake-shaped text in the file as decoded.
    ///
    /// This reports damage, not misdetection, and the difference matters. A
    /// windows-1252 file whose bytes happen to be valid UTF-8 is detected as
    /// UTF-8 and decodes to *clean* text — the mojibake shape sits in the
    /// windows-1252 reading, the one thrown away — and an ordinary UTF-8 file
    /// holding the same accented words has byte-for-byte identical content.
    /// Nothing in the bytes tells those two apart, so no warning here can;
    /// only --encoding settles it.
    ///
    /// What the shape does reliably catch is text that already went through the
    /// wrong encoding: UTF-8 that was double-encoded (`Ã©` for `é`), and a
    /// legacy file read under a correct explicit --encoding whose content was
    /// damaged before `intact` ever saw it. Both are the "do not hand-fix
    /// mojibake" condition, and neither was visible before.
    pub fn mojibake_warning(&self) -> Option<String> {
        let hint = self.mojibake()?;
        // No path: `info` prints one above already, and the write path prefixes
        // its own so a `batch` over several files stays attributable.
        let mut msg = format!(
            "{} mojibake-shaped sequence(s), first {:?} at line {}: text that was written \
             through the wrong encoding at some point. Report it rather than editing the \
             damaged text by hand.",
            hint.count,
            hint.sample,
            self.line_index.line_of_offset(hint.offset),
        );
        // A declared encoding makes the damage unambiguously pre-existing. An
        // inferred one leaves open that the reading itself is off, which would
        // change what the text actually says.
        if !matches!(self.detection, Detection::Explicit | Detection::Bom) {
            msg.push_str(&format!(
                " This file's encoding was inferred ({}), not declared - confirm it with \
                 --encoding LABEL before writing.",
                self.detection.as_str()
            ));
        }
        Some(msg)
    }

    /// Apply edits to the decoded text (used for previews and for computing the
    /// post-edit line count).
    pub fn apply_to_text(&self, edits: &[Edit]) -> String {
        let mut out = String::with_capacity(self.text.len());
        let mut cursor = 0usize;
        for edit in edits {
            out.push_str(&self.text[cursor..edit.start]);
            out.push_str(&edit.text);
            cursor = edit.end;
        }
        out.push_str(&self.text[cursor..]);
        out
    }

    /// Turn a list of edits into the new file bytes.
    pub fn build_output(
        &self,
        edits: &mut [Edit],
        policy: UnmappablePolicy,
        allow_lossy: bool,
    ) -> Result<Vec<u8>> {
        validate_edits(edits, self.text.len())?;

        if self.roundtrip {
            if let Some(map) = &self.map {
                return self.splice(edits, map, policy).map_err(|e| self.explain(e));
            }
        }

        if !allow_lossy {
            return Err(self.lossless_failure());
        }

        // Whole-file re-encode.
        let new_text = self.apply_to_text(edits);
        let mut out = Vec::new();
        if let Some(bom) = self.bom {
            out.extend_from_slice(bom.bytes());
        }
        out.extend_from_slice(&encode_text(self.encoding, &new_text, policy)?);
        Ok(out)
    }

    fn splice(
        &self,
        edits: &[Edit],
        map: &EncodedMap,
        policy: UnmappablePolicy,
    ) -> Result<Vec<u8>> {
        let base = self.content_start();
        let mut out = Vec::with_capacity(self.raw.len());
        out.extend_from_slice(&self.raw[..base]);
        let mut cursor = 0usize; // offset in decoded text

        for edit in edits {
            let from = map.to_encoded(edit.start).ok_or_else(|| {
                AppError::new(
                    ErrorKind::Other,
                    "internal: edit start is not on a character boundary",
                )
            })?;
            let cursor_enc = map.to_encoded(cursor).ok_or_else(|| {
                AppError::new(
                    ErrorKind::Other,
                    "internal: cursor is not on a character boundary",
                )
            })?;
            out.extend_from_slice(&self.raw[base + cursor_enc..base + from]);
            out.extend_from_slice(&encode_text(self.encoding, &edit.text, policy)?);
            cursor = edit.end;
        }
        let cursor_enc = map.to_encoded(cursor).ok_or_else(|| {
            AppError::new(
                ErrorKind::Other,
                "internal: cursor is not on a character boundary",
            )
        })?;
        out.extend_from_slice(&self.raw[base + cursor_enc..]);
        Ok(out)
    }

    /// Statistical detection is a guess, and a wrong guess among the single-byte
    /// encodings shows up here first: the existing bytes still round-trip, but a
    /// character you are inserting appears unrepresentable. Say so.
    fn explain(&self, err: AppError) -> AppError {
        if err.kind != ErrorKind::Encoding || self.detection != Detection::Guessed {
            return err;
        }
        AppError::new(
            err.kind,
            format!(
                "{} (this file's encoding was guessed, not declared)",
                err.message
            ),
        )
        .with_hint(format!(
            "if {} is not really the file's encoding, pass --encoding LABEL; otherwise convert the \
             file (`intact convert FILE --to utf-8`) or pass --unmappable replace|xml|skip",
            self.encoding.name()
        ))
    }

    fn lossless_failure(&self) -> AppError {
        let detail = if self.had_decode_errors {
            format!(
                "{} contains byte sequences that are not valid {}",
                self.path.display(),
                self.encoding.name()
            )
        } else if self.map.is_none() {
            format!(
                "{} uses the stateful encoding {}, which cannot be edited byte-for-byte",
                self.path.display(),
                self.encoding.name()
            )
        } else {
            format!(
                "{} does not round-trip through {}: re-encoding the decoded text would change untouched bytes",
                self.path.display(),
                self.encoding.name()
            )
        };
        AppError::new(ErrorKind::Encoding, format!("refusing to edit: {detail}")).with_hint(
            "pass --encoding LABEL if the encoding was guessed wrong, run `intact info FILE` to \
             inspect, or pass --lossy to rewrite the whole file anyway",
        )
    }

    /// Write bytes to the file, atomically, preserving permissions.
    pub fn save(&self, bytes: &[u8], backup: bool) -> Result<()> {
        if backup && self.existed {
            let mut bak = self.path.clone().into_os_string();
            bak.push(".bak");
            fs::write(&bak, &self.raw)?;
        }
        atomic_write(&self.path, bytes)
    }
}

fn validate_edits(edits: &mut [Edit], text_len: usize) -> Result<()> {
    edits.sort_by_key(|e| (e.start, e.end));
    let mut prev_end = 0usize;
    for edit in edits.iter() {
        if edit.start > edit.end || edit.end > text_len {
            return Err(AppError::new(
                ErrorKind::Other,
                "internal: edit range out of bounds",
            ));
        }
        if edit.start < prev_end {
            return Err(AppError::new(
                ErrorKind::Usage,
                "the requested edits overlap; apply them as separate commands",
            ));
        }
        prev_end = edit.end;
    }
    Ok(())
}

/// How many links to follow before declaring a loop, as the kernel does.
const MAX_LINK_DEPTH: usize = 40;

/// Follow a symlink chain to the file that should actually be rewritten.
///
/// Reading a file follows symlinks, so writing must too: renaming the temp
/// file over the link itself would replace the link with a regular file and
/// leave the real target untouched. A dangling link still resolves, so writing
/// through it creates the target it names.
fn resolve_write_target(path: &Path) -> Result<PathBuf> {
    let mut current = path.to_path_buf();
    for _ in 0..MAX_LINK_DEPTH {
        let is_link = fs::symlink_metadata(&current)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        if !is_link {
            return Ok(current);
        }
        let target = fs::read_link(&current)?;
        current = match current.parent() {
            Some(dir) if !target.is_absolute() && !dir.as_os_str().is_empty() => dir.join(target),
            _ => target,
        };
    }
    Err(AppError::new(
        ErrorKind::Io,
        format!("too many levels of symbolic links: {}", path.display()),
    ))
}

/// The file `path` really refers to, when `path` is a symlink: the file reads
/// and writes actually land on. `None` when it is an ordinary file, so callers
/// can report the indirection only when there is one.
pub fn link_target(path: &Path) -> Option<PathBuf> {
    let resolved = resolve_write_target(path).ok()?;
    (resolved != path).then_some(resolved)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    // Write to the file the path resolves to, never over a symlink to it.
    let path = &resolve_write_target(path)?;
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 + d.as_secs())
        .unwrap_or(0);
    let tmp = dir.join(format!(
        ".{file_name}.intact-{}-{}",
        std::process::id(),
        nanos
    ));

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)?;
    let result = (|| -> std::io::Result<()> {
        file.write_all(bytes)?;
        file.sync_all()
    })();
    drop(file);

    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(AppError::from(e));
    }

    // Carry over the original file's permissions.
    if let Ok(meta) = fs::metadata(path) {
        let _ = fs::set_permissions(&tmp, meta.permissions());
    }

    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(AppError::from(e));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoding_rs::WINDOWS_1252;

    fn doc_from(bytes: &[u8], enc: Option<&'static Encoding>) -> Document {
        Document::from_bytes(
            PathBuf::from("mem"),
            bytes.to_vec(),
            enc.map(ForcedEncoding::flag),
            true,
        )
    }

    #[test]
    fn latin1_roundtrips() {
        // "café\n" in windows-1252
        let bytes = b"caf\xE9\n";
        let doc = doc_from(bytes, Some(WINDOWS_1252));
        assert_eq!(doc.text, "café\n");
        assert!(doc.roundtrip);
    }

    #[test]
    fn splice_preserves_untouched_bytes() {
        // "café ré\n" in windows-1252; replace "café" with "thé".
        let bytes = b"caf\xE9 r\xE9\n";
        let doc = doc_from(bytes, Some(WINDOWS_1252));
        let mut edits = vec![Edit::new(0, "café".len(), "thé")];
        let out = doc
            .build_output(&mut edits, UnmappablePolicy::Error, false)
            .unwrap();
        assert_eq!(out, b"th\xE9 r\xE9\n".to_vec());
    }

    #[test]
    fn unmappable_character_is_refused() {
        let bytes = b"abc\n";
        let doc = doc_from(bytes, Some(WINDOWS_1252));
        let mut edits = vec![Edit::new(0, 3, "日本")];
        let err = doc
            .build_output(&mut edits, UnmappablePolicy::Error, false)
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Encoding);
    }

    #[test]
    fn unmappable_policies() {
        let bytes = b"abc\n";
        let doc = doc_from(bytes, Some(WINDOWS_1252));
        let mut edits = vec![Edit::new(0, 3, "x\u{65E5}y")];
        let out = doc
            .build_output(&mut edits.clone(), UnmappablePolicy::Replace, false)
            .unwrap();
        assert_eq!(out, b"x?y\n".to_vec());
        let out = doc
            .build_output(&mut edits.clone(), UnmappablePolicy::Xml, false)
            .unwrap();
        assert_eq!(out, b"x&#26085;y\n".to_vec());
        let out = doc
            .build_output(&mut edits, UnmappablePolicy::Skip, false)
            .unwrap();
        assert_eq!(out, b"xy\n".to_vec());
    }

    #[test]
    fn utf8_bom_is_preserved() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"hello\n");
        let doc = doc_from(&bytes, None);
        assert_eq!(doc.bom, Some(BomKind::Utf8));
        assert_eq!(doc.text, "hello\n");
        let mut edits = vec![Edit::new(0, 5, "bye")];
        let out = doc
            .build_output(&mut edits, UnmappablePolicy::Error, false)
            .unwrap();
        assert_eq!(out, [vec![0xEF, 0xBB, 0xBF], b"bye\n".to_vec()].concat());
    }

    #[test]
    fn utf16le_edits() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "héllo\n".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let doc = doc_from(&bytes, None);
        assert_eq!(doc.text, "héllo\n");
        assert!(doc.roundtrip);
        let mut edits = vec![Edit::new(0, "héllo".len(), "wörld")];
        let out = doc
            .build_output(&mut edits, UnmappablePolicy::Error, false)
            .unwrap();
        let mut expected = vec![0xFF, 0xFE];
        for unit in "wörld\n".encode_utf16() {
            expected.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(out, expected);
    }

    #[test]
    fn broken_bytes_are_refused_without_lossy() {
        // Lone 0x80 continuation byte: not valid UTF-8, forced as UTF-8.
        let doc = doc_from(b"ab\x80cd", Some(UTF_8));
        assert!(!doc.roundtrip);
        let mut edits = vec![Edit::new(0, 2, "xy")];
        let err = doc
            .build_output(&mut edits, UnmappablePolicy::Error, false)
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Encoding);
    }
}
