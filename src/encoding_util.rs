//! Encoding detection plus a character-granular encoder.
//!
//! `encoding_rs` is used for all legacy encodings, but its `encode()` helper
//! silently substitutes HTML numeric character references for characters the
//! target encoding cannot represent. For an editor that is data loss, so this
//! module drives the encoder one character at a time and surfaces every
//! unmappable character to the caller.

use clap::ValueEnum;
use encoding_rs::{
    Encoder, EncoderResult, Encoding, ISO_2022_JP, REPLACEMENT, UTF_16BE, UTF_16LE, UTF_8,
};

use crate::error::{AppError, ErrorKind, Result};

/// What to do with a character that the file's encoding cannot represent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum UnmappablePolicy {
    /// Refuse the edit (default): nothing is written.
    Error,
    /// Substitute `?`.
    Replace,
    /// Substitute an XML/HTML numeric character reference, e.g. `&#8364;`.
    Xml,
    /// Drop the character.
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BomKind {
    Utf8,
    Utf16Le,
    Utf16Be,
}

impl BomKind {
    pub fn bytes(self) -> &'static [u8] {
        match self {
            BomKind::Utf8 => &[0xEF, 0xBB, 0xBF],
            BomKind::Utf16Le => &[0xFF, 0xFE],
            BomKind::Utf16Be => &[0xFE, 0xFF],
        }
    }

    pub fn len(self) -> usize {
        self.bytes().len()
    }

    pub fn encoding(self) -> &'static Encoding {
        match self {
            BomKind::Utf8 => UTF_8,
            BomKind::Utf16Le => UTF_16LE,
            BomKind::Utf16Be => UTF_16BE,
        }
    }

    pub fn for_encoding(enc: &'static Encoding) -> Option<BomKind> {
        if enc == UTF_8 {
            Some(BomKind::Utf8)
        } else if enc == UTF_16LE {
            Some(BomKind::Utf16Le)
        } else if enc == UTF_16BE {
            Some(BomKind::Utf16Be)
        } else {
            None
        }
    }
}

/// Detect a byte-order mark. UTF-32 is not supported by `encoding_rs`, so a
/// `FF FE 00 00` prefix is deliberately reported as UTF-16LE only when it is
/// not a UTF-32LE BOM.
pub fn sniff_bom(bytes: &[u8]) -> Option<BomKind> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        Some(BomKind::Utf8)
    } else if bytes.starts_with(&[0xFF, 0xFE]) && !bytes.starts_with(&[0xFF, 0xFE, 0x00, 0x00]) {
        Some(BomKind::Utf16Le)
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        Some(BomKind::Utf16Be)
    } else {
        None
    }
}

/// Resolve a user-supplied encoding label ("latin1", "cp1252", "utf-8", ...).
pub fn encoding_for_label(label: &str) -> Result<&'static Encoding> {
    let trimmed = label.trim();
    if let Some(enc) = Encoding::for_label(trimmed.as_bytes()) {
        if enc == REPLACEMENT {
            return Err(AppError::new(
                ErrorKind::Usage,
                format!("encoding label '{label}' maps to the WHATWG 'replacement' encoding, which cannot represent text"),
            ));
        }
        return Ok(enc);
    }
    Err(AppError::new(
        ErrorKind::Usage,
        format!("unknown encoding label '{label}'"),
    )
    .with_hint("run `intact encodings` to list supported labels"))
}

/// Encodings whose encoder carries state across characters. Splicing individual
/// regions of the file is unsound for these, so the whole file is re-encoded.
pub fn is_stateful(enc: &'static Encoding) -> bool {
    enc == ISO_2022_JP
}

/// Streaming encoder that reports unmappable characters instead of silently
/// substituting for them.
pub struct StreamEncoder {
    inner: Inner,
}

enum Inner {
    Utf8,
    Utf16 { be: bool },
    Legacy(Box<Encoder>),
}

impl StreamEncoder {
    pub fn new(enc: &'static Encoding) -> Self {
        let inner = if enc == UTF_8 {
            Inner::Utf8
        } else if enc == UTF_16LE {
            Inner::Utf16 { be: false }
        } else if enc == UTF_16BE {
            Inner::Utf16 { be: true }
        } else {
            Inner::Legacy(Box::new(enc.new_encoder()))
        };
        StreamEncoder { inner }
    }

    /// Encode one character, appending to `out`.
    /// Returns `Err(ch)` if the character has no representation in the target
    /// encoding; any bytes emitted before the failure are still appended.
    pub fn push_char(&mut self, ch: char, out: &mut Vec<u8>) -> std::result::Result<(), char> {
        match &mut self.inner {
            Inner::Utf8 => {
                let mut tmp = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut tmp).as_bytes());
                Ok(())
            }
            Inner::Utf16 { be } => {
                let mut tmp = [0u16; 2];
                for unit in ch.encode_utf16(&mut tmp) {
                    if *be {
                        out.extend_from_slice(&unit.to_be_bytes());
                    } else {
                        out.extend_from_slice(&unit.to_le_bytes());
                    }
                }
                Ok(())
            }
            Inner::Legacy(encoder) => {
                let mut chbuf = [0u8; 4];
                let s: &str = ch.encode_utf8(&mut chbuf);
                // Comfortably larger than any single-character output produced
                // by the encodings encoding_rs supports.
                let mut buf = [0u8; 32];
                let (result, _read, written) =
                    encoder.encode_from_utf8_without_replacement(s, &mut buf, false);
                out.extend_from_slice(&buf[..written]);
                match result {
                    EncoderResult::InputEmpty => Ok(()),
                    EncoderResult::Unmappable(c) => Err(c),
                    EncoderResult::OutputFull => {
                        // Not reachable with a 32-byte buffer and one character.
                        Err(ch)
                    }
                }
            }
        }
    }

    /// Flush any encoder state (e.g. the ISO-2022-JP shift back to ASCII).
    pub fn finish(&mut self, out: &mut Vec<u8>) {
        if let Inner::Legacy(encoder) = &mut self.inner {
            let mut buf = [0u8; 32];
            let (_result, _read, written) =
                encoder.encode_from_utf8_without_replacement("", &mut buf, true);
            out.extend_from_slice(&buf[..written]);
        }
    }
}

/// Encode a whole string under the given unmappable-character policy.
pub fn encode_text(
    enc: &'static Encoding,
    text: &str,
    policy: UnmappablePolicy,
) -> Result<Vec<u8>> {
    let mut encoder = StreamEncoder::new(enc);
    let mut out = Vec::with_capacity(text.len());
    for ch in text.chars() {
        if encoder.push_char(ch, &mut out).is_err() {
            match policy {
                UnmappablePolicy::Error => {
                    return Err(AppError::new(
                        ErrorKind::Encoding,
                        format!(
                            "character {:?} (U+{:04X}) cannot be represented in {}",
                            ch,
                            ch as u32,
                            enc.name()
                        ),
                    )
                    .with_hint(
                        "convert the file first (`intact convert FILE --to utf-8`) or pass \
                         --unmappable replace|xml|skip",
                    ));
                }
                UnmappablePolicy::Replace => {
                    let _ = encoder.push_char('?', &mut out);
                }
                UnmappablePolicy::Xml => {
                    for c in format!("&#{};", ch as u32).chars() {
                        let _ = encoder.push_char(c, &mut out);
                    }
                }
                UnmappablePolicy::Skip => {}
            }
        }
    }
    encoder.finish(&mut out);
    Ok(out)
}

/// Per-character byte offsets of `text` re-encoded in `enc`.
pub struct EncodedMap {
    /// Byte offset of each character within `text` (UTF-8), plus a final
    /// sentinel equal to `text.len()`.
    pub utf8_offsets: Vec<u32>,
    /// Byte offset of the same character within the encoded output, plus a
    /// final sentinel equal to `encoded.len()`.
    pub encoded_offsets: Vec<u32>,
    pub encoded: Vec<u8>,
}

impl EncodedMap {
    /// Translate a UTF-8 offset in the decoded text to an offset in the encoded
    /// bytes. The offset must sit on a character boundary.
    pub fn to_encoded(&self, utf8_offset: usize) -> Option<usize> {
        let needle = u32::try_from(utf8_offset).ok()?;
        match self.utf8_offsets.binary_search(&needle) {
            Ok(idx) => Some(self.encoded_offsets[idx] as usize),
            Err(_) => None,
        }
    }
}

/// Re-encode `text` character by character, recording offsets. Returns `None`
/// if any character is unmappable (the caller then treats the document as not
/// round-trippable).
pub fn build_encoded_map(enc: &'static Encoding, text: &str) -> Option<EncodedMap> {
    let mut encoder = StreamEncoder::new(enc);
    let mut encoded = Vec::with_capacity(text.len());
    let mut utf8_offsets = Vec::with_capacity(text.chars().count() + 1);
    let mut encoded_offsets = Vec::with_capacity(text.chars().count() + 1);

    for (idx, ch) in text.char_indices() {
        utf8_offsets.push(idx as u32);
        encoded_offsets.push(encoded.len() as u32);
        if encoder.push_char(ch, &mut encoded).is_err() {
            return None;
        }
    }
    encoder.finish(&mut encoded);
    utf8_offsets.push(text.len() as u32);
    encoded_offsets.push(encoded.len() as u32);

    Some(EncodedMap {
        utf8_offsets,
        encoded_offsets,
        encoded,
    })
}

/// Guess the encoding of bytes that are not valid UTF-8 and carry no BOM.
pub fn detect_legacy(bytes: &[u8]) -> &'static Encoding {
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(bytes, true);
    detector.guess(None, true)
}

/// Labels advertised by `intact encodings`.
pub const KNOWN_LABELS: &[&str] = &[
    "utf-8",
    "utf-16le",
    "utf-16be",
    "windows-1250",
    "windows-1251",
    "windows-1252",
    "windows-1253",
    "windows-1254",
    "windows-1255",
    "windows-1256",
    "windows-1257",
    "windows-1258",
    "windows-874",
    "iso-8859-2",
    "iso-8859-3",
    "iso-8859-4",
    "iso-8859-5",
    "iso-8859-6",
    "iso-8859-7",
    "iso-8859-8",
    "iso-8859-8-i",
    "iso-8859-10",
    "iso-8859-13",
    "iso-8859-14",
    "iso-8859-15",
    "iso-8859-16",
    "koi8-r",
    "koi8-u",
    "macintosh",
    "x-mac-cyrillic",
    "ibm866",
    "gbk",
    "gb18030",
    "big5",
    "euc-jp",
    "shift_jis",
    "iso-2022-jp",
    "euc-kr",
];
