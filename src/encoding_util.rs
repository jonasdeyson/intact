//! Encoding detection plus a character-granular encoder.
//!
//! `encoding_rs` is used for all legacy encodings, but its `encode()` helper
//! silently substitutes HTML numeric character references for characters the
//! target encoding cannot represent. For an editor that is data loss, so this
//! module drives the encoder one character at a time and surfaces every
//! unmappable character to the caller.

use clap::ValueEnum;
use encoding_rs::{
    Encoder, EncoderResult, Encoding, ISO_2022_JP, REPLACEMENT, UTF_8, UTF_16BE, UTF_16LE,
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

/// The character set a document is read and written in.
///
/// For every encoding but one this is just `encoding_rs`'s `Encoding`. ASCII is
/// the exception, because the WHATWG standard has no ASCII encoding: `ascii`,
/// `us-ascii` and `ansi_x3.4-1968` are all labels *for windows-1252*. Resolving
/// them that way would make `--encoding ascii` a way of asking for
/// windows-1252 under a name that promises the opposite - an edit inserting `é`
/// would be accepted and written as the byte 0xE9, which is ASCII in no sense
/// at all. So ASCII is carried here as its own charset: identical to UTF-8 over
/// U+0000..=U+007F and holding nothing above it, which turns declaring it into
/// a mandate the encoder enforces rather than a synonym for a superset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Charset {
    enc: &'static Encoding,
    /// Refuse every character above U+007F, whatever `enc` could represent.
    ascii_only: bool,
}

impl Charset {
    pub const ASCII: Charset = Charset {
        enc: UTF_8,
        ascii_only: true,
    };

    pub const fn new(enc: &'static Encoding) -> Charset {
        Charset {
            enc,
            ascii_only: false,
        }
    }

    /// The underlying `encoding_rs` encoding. ASCII reports UTF-8, which agrees
    /// with it on every character ASCII has; the difference is only in what is
    /// refused, and that is [`Charset::is_ascii_only`]'s business.
    pub fn encoding(self) -> &'static Encoding {
        self.enc
    }

    pub fn is_ascii_only(self) -> bool {
        self.ascii_only
    }

    pub fn name(self) -> &'static str {
        if self.ascii_only {
            "US-ASCII"
        } else {
            self.enc.name()
        }
    }

    /// The name as a `--encoding` label, for hints that quote one back.
    pub fn label(self) -> String {
        self.name().to_lowercase()
    }

    /// Decode file bytes, reporting whether anything was undecodable. Any BOM
    /// has already been split off by the caller.
    pub fn decode_without_bom_handling(self, bytes: &[u8]) -> (String, bool) {
        if !self.ascii_only {
            let (cow, had_errors) = self.enc.decode_without_bom_handling(bytes);
            return (cow.into_owned(), had_errors);
        }
        // A byte above 0x7F is not ASCII, so under a declared ASCII it is
        // undecodable — one U+FFFD each, as encoding_rs does for a malformed
        // sequence. That flags the file as not round-trippable, so an edit to
        // it is refused until the caller declares what it really is.
        if bytes.is_ascii() {
            return (String::from_utf8_lossy(bytes).into_owned(), false);
        }
        let mut out = String::with_capacity(bytes.len());
        for &b in bytes {
            out.push(if b.is_ascii() {
                b as char
            } else {
                char::REPLACEMENT_CHARACTER
            });
        }
        (out, true)
    }
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

    pub fn charset(self) -> Charset {
        Charset::new(self.encoding())
    }

    /// The BOM that belongs to `cs`, if it has one. ASCII does not: a BOM is
    /// U+FEFF, which ASCII cannot hold, and its bytes in any encoding are above
    /// 0x7F.
    pub fn for_charset(cs: Charset) -> Option<BomKind> {
        if cs.is_ascii_only() {
            return None;
        }
        let enc = cs.encoding();
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

/// Labels that name ASCII itself. WHATWG resolves every one of these to
/// windows-1252, so they are intercepted before `Encoding::for_label` and mean
/// what they say instead. See [`Charset`].
const ASCII_LABELS: &[&str] = &[
    "ascii",
    "us-ascii",
    "us_ascii",
    "usascii",
    "ansi_x3.4-1968",
    "ansi_x3.4-1986",
    "iso-ir-6",
    "iso646-us",
    "cp367",
    "ibm367",
    "csascii",
];

/// Resolve a user-supplied encoding label ("latin1", "cp1252", "utf-8", ...).
pub fn encoding_for_label(label: &str) -> Result<Charset> {
    let trimmed = label.trim();
    if ASCII_LABELS.iter().any(|l| trimmed.eq_ignore_ascii_case(l)) {
        return Ok(Charset::ASCII);
    }
    if let Some(enc) = Encoding::for_label(trimmed.as_bytes()) {
        if enc == REPLACEMENT {
            return Err(AppError::new(
                ErrorKind::Usage,
                format!(
                    "encoding label '{label}' maps to the WHATWG 'replacement' encoding, which cannot represent text"
                ),
            ));
        }
        return Ok(Charset::new(enc));
    }
    Err(AppError::new(
        ErrorKind::Usage,
        format!("unknown encoding label '{label}'"),
    )
    .with_hint("run `intact guide encoding` to list supported labels"))
}

/// Encodings whose encoder carries state across characters. Splicing individual
/// regions of the file is unsound for these, so the whole file is re-encoded.
pub fn is_stateful(cs: Charset) -> bool {
    !cs.is_ascii_only() && cs.encoding() == ISO_2022_JP
}

/// Streaming encoder that reports unmappable characters instead of silently
/// substituting for them.
pub struct StreamEncoder {
    inner: Inner,
}

enum Inner {
    Ascii,
    Utf8,
    Utf16 { be: bool },
    Legacy(Box<Encoder>),
}

impl StreamEncoder {
    pub fn new(cs: Charset) -> Self {
        let enc = cs.encoding();
        let inner = if cs.is_ascii_only() {
            Inner::Ascii
        } else if enc == UTF_8 {
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
            Inner::Ascii => {
                if ch.is_ascii() {
                    out.push(ch as u8);
                    Ok(())
                } else {
                    Err(ch)
                }
            }
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
pub fn encode_text(cs: Charset, text: &str, policy: UnmappablePolicy) -> Result<Vec<u8>> {
    let mut encoder = StreamEncoder::new(cs);
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
                            cs.name()
                        ),
                    )
                    .with_hint(if cs.is_ascii_only() {
                        // ASCII is only ever in play because someone asked for
                        // it by name, so the fix is a different --encoding, not
                        // a conversion of the file.
                        "US-ASCII holds nothing above U+007F: pass an encoding that has this \
                         character (--encoding utf-8, or the file's real legacy encoding), or \
                         --unmappable replace|xml|skip"
                    } else {
                        "convert the file first (`intact convert FILE --to utf-8`) or pass \
                         --unmappable replace|xml|skip"
                    }));
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
pub fn build_encoded_map(cs: Charset, text: &str) -> Option<EncodedMap> {
    let mut encoder = StreamEncoder::new(cs);
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
pub fn detect_legacy(bytes: &[u8]) -> Charset {
    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Allow);
    detector.feed(bytes, true);
    Charset::new(detector.guess(None, chardetng::Utf8Detection::Allow))
}

/// A run of text shaped like mojibake: UTF-8 bytes read through a single-byte
/// encoding, so `é` (C3 A9) reads as `Ã©` and `'` (E2 80 99) as `â€™`.
#[derive(Debug, Clone)]
pub struct MojibakeHint {
    /// How many such sequences the text contains.
    pub count: usize,
    /// Byte offset of the first one within the decoded text.
    pub offset: usize,
    /// The first sequence itself, for quoting back at the user.
    pub sample: String,
}

/// The characters windows-1252 stores in 0x80..=0x9F. Together with
/// U+00A0..=U+00BF, which it stores as themselves, these are exactly the
/// characters that a UTF-8 continuation byte decodes to.
const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
];

/// Whether `ch` is one a UTF-8 continuation byte (0x80..=0xBF) decodes to.
fn is_continuation_shaped(ch: char) -> bool {
    matches!(ch, '\u{00A0}'..='\u{00BF}') || CP1252_HIGH.contains(&ch)
}

/// Find mojibake-shaped sequences in decoded text.
///
/// Only the three lead characters that dominate real mojibake are considered,
/// and each must be followed by a continuation-shaped character. That pairing
/// is what keeps the check quiet on genuine text: `Ã` and `Â` occur in
/// Portuguese and French before ASCII letters, never before U+00A0..=U+00BF,
/// and `â` is only counted before `€` - the `â€™`/`â€œ` family - so `château`
/// does not register.
pub fn scan_mojibake(text: &str) -> Option<MojibakeHint> {
    let mut count = 0usize;
    let mut first: Option<(usize, usize)> = None;

    for (idx, ch) in text.char_indices() {
        if !matches!(ch, 'Ã' | 'Â' | 'â') {
            continue;
        }
        // Walking forward from the lead keeps the look-ahead honest: `chars`
        // starts at the character after it, so `next` and `third` are distinct.
        let mut chars = text[idx + ch.len_utf8()..].chars();
        let Some(next) = chars.next() else { continue };
        let matched = match ch {
            'Ã' | 'Â' => is_continuation_shaped(next),
            'â' => next == '\u{20AC}',
            _ => false,
        };
        if !matched {
            continue;
        }
        count += 1;
        if first.is_none() {
            // Take the trailing character and the one after it when it is also
            // continuation-shaped, so the sample reads as the whole garbled
            // cluster (`â€™`, not `â€`). Three characters is the longest a
            // single mis-decoded scalar produces.
            let mut end = idx + ch.len_utf8() + next.len_utf8();
            if let Some(third) = chars.next() {
                if is_continuation_shaped(third) {
                    end += third.len_utf8();
                }
            }
            first = Some((idx, end));
        }
    }

    first.map(|(start, end)| MojibakeHint {
        count,
        offset: start,
        sample: text[start..end].to_string(),
    })
}

/// Labels advertised by `intact guide encoding`.
pub const KNOWN_LABELS: &[&str] = &[
    "ascii",
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
