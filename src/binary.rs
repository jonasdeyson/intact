//! Judging whether a file is text, before anything decodes it.
//!
//! The check is a bounded prefix read as code units, so it costs nothing on
//! a large file and can run ahead of the decode it guards.

use encoding_rs::{Encoding, UTF_16BE, UTF_16LE};

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
    pub(crate) fn refusal_hint(self) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use encoding_rs::UTF_8;

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
}
