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

use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE};

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

// ------------------------------------------------------------ binary sniff

/// Why a file was judged not to be text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryHint {
    /// A NUL, at this byte offset.
    Nul { offset: usize },
    /// A NUL every other byte: not binary at all, but UTF-16 with no BOM,
    /// read as single bytes because nothing declared the encoding. `be` is
    /// which half of each pair holds the NUL.
    Utf16NoBom { be: bool },
    /// An unpaired surrogate code unit, at this byte offset. Only UTF-16 files
    /// are tested for this, and well-formed ones never contain one.
    Surrogate { offset: usize },
    /// `count` stray control characters among the `total` read from the
    /// sniffed prefix (bytes, or UTF-16 code units).
    Controls { count: usize, total: usize },
}

impl BinaryHint {
    /// Stable tag for `--json`.
    pub fn reason(self) -> &'static str {
        match self {
            BinaryHint::Nul { .. } => "nul",
            BinaryHint::Utf16NoBom { .. } => "utf-16-no-bom",
            BinaryHint::Surrogate { .. } => "unpaired-surrogate",
            BinaryHint::Controls { .. } => "controls",
        }
    }

    /// The byte offset the judgement rests on, where it rests on one.
    pub fn offset(self) -> Option<usize> {
        match self {
            BinaryHint::Nul { offset } | BinaryHint::Surrogate { offset } => Some(offset),
            BinaryHint::Utf16NoBom { .. } | BinaryHint::Controls { .. } => None,
        }
    }

    /// What the caller should do about it. Only the UTF-16 case has a real
    /// answer; the rest can only be overridden.
    pub fn fix(self) -> String {
        match self {
            BinaryHint::Utf16NoBom { be } => format!(
                "pass --encoding {} to read it as the text it is, or --force to take it as bytes",
                if be { "utf-16be" } else { "utf-16le" }
            ),
            _ => "pass --force to use it anyway".to_string(),
        }
    }

    /// The same, for a command that has just refused the file — where
    /// `info` is worth naming, since it is the way to see more.
    fn refusal_hint(self) -> String {
        match self {
            BinaryHint::Utf16NoBom { .. } => self.fix(),
            _ => format!("run `intact info FILE` to inspect it, or {}", self.fix()),
        }
    }

    pub fn describe(self) -> String {
        match self {
            BinaryHint::Nul { offset } => format!("NUL byte at offset {offset} (0x{offset:X})"),
            BinaryHint::Utf16NoBom { be } => format!(
                "a NUL every other byte, which is how UTF-16{} with no BOM reads as single bytes",
                if be { "BE" } else { "LE" }
            ),
            BinaryHint::Surrogate { offset } => {
                format!("unpaired UTF-16 surrogate at offset {offset} (0x{offset:X})")
            }
            BinaryHint::Controls { count, total } => format!(
                "{}% control characters ({count} of the first {total})",
                count * 100 / total.max(1)
            ),
        }
    }
}

/// Whether the caller will accept a file that does not look like text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryPolicy {
    /// Refuse it before decoding. The default for every command but `info`.
    Refuse,
    /// Load it anyway: `--force`, or `info`, which exists to explain refusals.
    Allow,
}

/// How much of the file the sniff reads. The same 8 KiB prefix git uses: a NUL
/// 40 MB in says less about a file than its header does, and a bounded prefix
/// keeps the check cheap enough to run *before* decoding, which is the point —
/// decoding a large blob builds a character offset table costing several bytes
/// per character, and there is no reason to pay that for a file about to be
/// refused.
const SNIFF_WINDOW: usize = 8192;

/// The share of stray control characters above which a prefix is called
/// binary. High-entropy data (compressed, encrypted, machine code) carries
/// around 10% by chance, 27 of the 256 byte values being stray controls.
const CONTROL_PERCENT: usize = 5;

/// Below this many, a ratio over so short a prefix means nothing: a two-byte
/// file holding one stray control is 50% control and still probably text.
const CONTROL_MIN: usize = 4;

/// Whether `c` is a control character that carries no meaning in text. Tab,
/// LF, form feed and CR are ordinary layout, and ESC is how ISO-2022-JP
/// switches character sets.
fn is_stray_control(c: u32) -> bool {
    c < 0x20 && !matches!(c, 0x09 | 0x0A | 0x0C | 0x0D | 0x1B)
}

fn control_verdict(count: usize, total: usize) -> Option<BinaryHint> {
    if count >= CONTROL_MIN && count * 100 > total * CONTROL_PERCENT {
        Some(BinaryHint::Controls { count, total })
    } else {
        None
    }
}

/// Judge whether bytes are text, without decoding them.
///
/// `enc` is the encoding known *before* decoding, and only UTF-16 changes the
/// reading — but it changes it completely. Every other ASCII byte of UTF-16
/// text is a NUL, so a byte-oriented test rejects every UTF-16 file; and a
/// binary file that happens to begin `FF FE` is taken for UTF-16, so a test
/// that simply exempts UTF-16 lets it through. That exemption is what the
/// previous NUL-only check did, and it is why this reads code units instead.
pub fn sniff_binary(raw: &[u8], enc: &'static Encoding) -> Option<BinaryHint> {
    let window = &raw[..raw.len().min(SNIFF_WINDOW)];
    if enc == UTF_16LE || enc == UTF_16BE {
        sniff_utf16(window, enc == UTF_16BE)
    } else {
        sniff_bytes(window)
    }
}

/// How many byte pairs the BOM-less-UTF-16 check looks at, and the share of
/// them that must show the alternating NUL for it to say so.
const UTF16_PROBE_PAIRS: usize = 128;
const UTF16_PROBE_PERCENT: usize = 80;

/// Whether a NUL-bearing prefix is really UTF-16 that nothing declared.
///
/// Worth separating from plain binary because it is the one case with a fix
/// rather than an override: the file *is* text, and `--encoding utf-16le`
/// reads it. Detection cannot find it unaided — chardetng does not guess
/// UTF-16, so a BOM-less UTF-16 file is guessed as some single-byte encoding
/// and decodes to text interleaved with NULs.
///
/// The test is the alternating NUL itself. One half of each pair must be NUL
/// far more often than not, and the other half never, which ordinary binary
/// data does not manage for long.
fn utf16_without_bom(window: &[u8]) -> Option<BinaryHint> {
    let pairs = window.chunks_exact(2).take(UTF16_PROBE_PAIRS);
    let (mut le, mut be, mut n) = (0usize, 0usize, 0usize);
    for pair in pairs {
        n += 1;
        match (pair[0] == 0, pair[1] == 0) {
            // A NUL in both halves is U+0000, which is not text in any
            // encoding, so it counts for neither reading.
            (true, true) => {}
            (false, true) => le += 1,
            (true, false) => be += 1,
            (false, false) => {}
        }
    }
    if n < 2 {
        return None;
    }
    let threshold = n * UTF16_PROBE_PERCENT / 100;
    if le > threshold && be == 0 {
        Some(BinaryHint::Utf16NoBom { be: false })
    } else if be > threshold && le == 0 {
        Some(BinaryHint::Utf16NoBom { be: true })
    } else {
        None
    }
}

fn sniff_bytes(window: &[u8]) -> Option<BinaryHint> {
    if let Some(offset) = window.iter().position(|&b| b == 0) {
        return Some(utf16_without_bom(window).unwrap_or(BinaryHint::Nul { offset }));
    }
    let count = window
        .iter()
        .filter(|&&b| is_stray_control(u32::from(b)))
        .count();
    control_verdict(count, window.len())
}

/// The UTF-16 reading. Stray controls are a much weaker signal here — random
/// data lands on a control unit only about once in 2400 units, where it lands
/// on a surrogate about once in 32 — so the decisive test is pairing: a lone
/// surrogate cannot occur in well-formed UTF-16, and binary data read as
/// UTF-16 produces one within a few hundred units with near certainty.
fn sniff_utf16(window: &[u8], be: bool) -> Option<BinaryHint> {
    let units: Vec<u16> = window
        .chunks_exact(2)
        .map(|p| {
            if be {
                u16::from_be_bytes([p[0], p[1]])
            } else {
                u16::from_le_bytes([p[0], p[1]])
            }
        })
        .collect();

    let mut controls = 0usize;
    let mut i = 0usize;
    while i < units.len() {
        let unit = units[i];
        if unit == 0 {
            return Some(BinaryHint::Nul { offset: i * 2 });
        }
        if (0xD800..=0xDBFF).contains(&unit) {
            // A high surrogate must be followed by a low one. A high surrogate
            // in the final position of the window is not evidence of anything:
            // its partner may simply sit past the prefix we read.
            match units.get(i + 1) {
                Some(next) if (0xDC00..=0xDFFF).contains(next) => {
                    i += 2;
                    continue;
                }
                Some(_) => return Some(BinaryHint::Surrogate { offset: i * 2 }),
                None => break,
            }
        }
        if (0xDC00..=0xDFFF).contains(&unit) {
            // A low surrogate reached here was not consumed as the second half
            // of a pair, so it stands alone.
            return Some(BinaryHint::Surrogate { offset: i * 2 });
        }
        if is_stray_control(u32::from(unit)) {
            controls += 1;
        }
        i += 1;
    }
    control_verdict(controls, units.len())
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
    /// Why the file does not look like text, if it does not. Only ever `Some`
    /// on a document loaded under `BinaryPolicy::Allow`.
    pub binary: Option<BinaryHint>,
    map: Option<EncodedMap>,
    line_index: LineIndex,
}

impl Document {
    /// Read a file from disk. `encoding` overrides detection.
    pub fn load(
        path: &Path,
        encoding: Option<ForcedEncoding>,
        binary: BinaryPolicy,
    ) -> Result<Document> {
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
        gate_binary(path, &raw, encoding, binary)?;
        Ok(Document::from_bytes(
            path.to_path_buf(),
            raw,
            encoding,
            existed,
        ))
    }

    /// Load a file that is allowed to be missing (for `write` / `create`).
    pub fn load_or_empty(
        path: &Path,
        encoding: Option<ForcedEncoding>,
        binary: BinaryPolicy,
    ) -> Result<Document> {
        match fs::read(path) {
            Ok(bytes) => {
                gate_binary(path, &bytes, encoding, binary)?;
                Ok(Document::from_bytes(
                    path.to_path_buf(),
                    bytes,
                    encoding,
                    true,
                ))
            }
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
            binary: sniff_binary(&raw, encoding),
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

    /// True if the file does not look like text. See [`sniff_binary`].
    pub fn looks_binary(&self) -> bool {
        self.binary.is_some()
    }

    /// Mojibake-shaped sequences in the decoded text, if any. Computed on
    /// demand: only `info` and the write path ask, so the read-only commands
    /// should not pay for another pass over the text.
    ///
    /// Never reported for a file that is not text. The shape is two ordinary
    /// bytes in sequence, so blobs turn it up constantly by chance — a real
    /// `/bin/ls` yields 93 of them, `libc.so.6` 1581 — and the warning it
    /// drives tells the reader to report damaged text rather than hand-fix it,
    /// which is not advice about an ELF file. The binary verdict already says
    /// the decoded reading means nothing; a second warning drawn from that
    /// same reading adds no information and contradicts the first.
    pub fn mojibake(&self) -> Option<MojibakeHint> {
        if self.binary.is_some() {
            return None;
        }
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

/// The encoding known before anything is decoded: whatever the caller forced,
/// else whatever a BOM declares. That is all the sniff needs, because the only
/// distinction it draws is UTF-16 against everything else, and a BOM-less
/// UTF-16 file is not a case that arises — chardetng never guesses UTF-16, so
/// full detection would reach the same branch this does.
fn early_encoding(raw: &[u8], forced: Option<ForcedEncoding>) -> &'static Encoding {
    forced
        .map(|f| f.encoding)
        .or_else(|| sniff_bom(raw).map(|b| b.encoding()))
        .unwrap_or(UTF_8)
}

/// Refuse a file that does not look like text, before it is decoded.
///
/// This guards reading as well as writing. Writing a binary file is the
/// obvious hazard, but it is the better defended one: an edit only reaches the
/// bytes through the round-trip check, which most binaries fail. Reading is
/// the accident that actually happens — a glob that catches a `.png`, and
/// `view` pipes NULs and escape sequences into a terminal or an agent's
/// context, having reported nothing wrong.
fn gate_binary(
    path: &Path,
    raw: &[u8],
    forced: Option<ForcedEncoding>,
    policy: BinaryPolicy,
) -> Result<()> {
    if policy == BinaryPolicy::Allow {
        return Ok(());
    }
    let Some(hint) = sniff_binary(raw, early_encoding(raw, forced)) else {
        return Ok(());
    };
    Err(AppError::new(
        ErrorKind::Encoding,
        format!(
            "{} does not look like a text file: {}",
            path.display(),
            hint.describe()
        ),
    )
    .with_hint(hint.refusal_hint()))
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

    /// Deterministic stand-in for high-entropy data: a linear congruential
    /// sequence, so the test does not depend on a random source.
    fn pseudo_random(n: usize) -> Vec<u8> {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        (0..n)
            .map(|_| {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (state >> 33) as u8
            })
            .collect()
    }

    #[test]
    fn plain_text_is_not_binary() {
        assert_eq!(sniff_binary("café\nrésumé\n".as_bytes(), UTF_8), None);
        assert_eq!(sniff_binary(b"caf\xE9\r\n\ttabbed\x0C\n", UTF_8), None);
        assert_eq!(sniff_binary(b"", UTF_8), None);
    }

    #[test]
    fn a_nul_is_binary_and_says_where() {
        assert_eq!(
            sniff_binary(b"abc\x00def", UTF_8),
            Some(BinaryHint::Nul { offset: 3 })
        );
    }

    /// The gap the previous NUL-only check left: high-entropy data that
    /// happens to carry no NUL in its first 8 KiB sailed straight through.
    #[test]
    fn binary_without_a_nul_is_still_binary() {
        let data: Vec<u8> = pseudo_random(4000)
            .into_iter()
            .filter(|&b| b != 0)
            .collect();
        assert!(matches!(
            sniff_binary(&data, UTF_8),
            Some(BinaryHint::Controls { .. })
        ));
    }

    /// A stray control or two does not condemn a file; ratio and count both
    /// have to clear their thresholds.
    #[test]
    fn a_few_stray_controls_are_tolerated() {
        let mut text = b"a normal line of prose, long enough to dilute the controls\n".to_vec();
        text.extend_from_slice(b"\x01\x02");
        assert_eq!(sniff_binary(&text, UTF_8), None);
    }

    #[test]
    fn utf16_text_is_not_binary() {
        let bytes: Vec<u8> = "héllo wörld\n"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        assert_eq!(sniff_binary(&bytes, UTF_16LE), None);
    }

    /// The other half of the old gap: UTF-16 was exempted wholesale, so a
    /// binary file beginning `FF FE` was declared text without being looked at.
    #[test]
    fn binary_read_as_utf16_is_caught_by_pairing() {
        let data = pseudo_random(2000);
        assert!(matches!(
            sniff_binary(&data, UTF_16LE),
            Some(BinaryHint::Surrogate { .. })
        ));
    }

    /// BOM-less UTF-16 is text, not binary, and saying so is what turns the
    /// refusal into a fixable one.
    #[test]
    fn bom_less_utf16_is_named_as_such() {
        let le: Vec<u8> = "hello world, a line of text\n"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        assert_eq!(
            sniff_binary(&le, UTF_8),
            Some(BinaryHint::Utf16NoBom { be: false })
        );
        let be: Vec<u8> = "hello world, a line of text\n"
            .encode_utf16()
            .flat_map(|u| u.to_be_bytes())
            .collect();
        assert_eq!(
            sniff_binary(&be, UTF_8),
            Some(BinaryHint::Utf16NoBom { be: true })
        );
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
