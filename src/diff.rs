//! Line diffs for edit previews, in unified-diff form.
//!
//! Two sources produce the same hunks, and share the grouping and rendering
//! below. [`from_edits`] derives them from the exact spans intact is about to
//! splice, so nothing is inferred: the lines it reports as changed are the
//! lines those spans cover. [`from_texts`] aligns the two texts with `similar`,
//! for the two paths that have no single edit list to work from — `batch`,
//! which re-edits the document between steps, and `convert`, which rewrites
//! terminators across the whole file.
//!
//! Line *terminators* are deliberately not part of the compared content: a file
//! converted from LF to CRLF would otherwise show up as every line changed,
//! which is unreadable. They are reported separately, as an [`EolChange`], so
//! that "would change" is never left unexplained.

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use similar::{Algorithm, DiffOp, TextDiff};

use crate::document::Edit;
use crate::lines::{self, LineIndex};

/// Unchanged lines shown either side of a change.
pub const DEFAULT_CONTEXT: usize = 3;
/// Rows beyond this are dropped from human output, with a note saying so.
pub const MAX_RENDERED_ROWS: usize = 400;
/// How long the text-to-text alignment may take before it settles for a
/// non-minimal answer.
const DIFF_DEADLINE: Duration = Duration::from_millis(500);
/// Longest `before`/`after` excerpt reported per edit in JSON.
const MAX_EDIT_TEXT: usize = 400;
/// Most edits itemised in JSON.
const MAX_EDITS_REPORTED: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Context(String),
    Removed(String),
    Added(String),
    /// `\ No newline at end of file`, qualifying the row before it. Not counted
    /// in the hunk's line totals, as in every other unified diff.
    NoEol,
}

#[derive(Debug)]
pub struct Hunk {
    pub old_start: usize,
    pub old_len: usize,
    pub new_start: usize,
    pub new_len: usize,
    pub rows: Vec<Row>,
}

/// Line-terminator counts on either side, when they are not the same.
#[derive(Debug, Clone, Copy)]
pub struct EolChange {
    pub before: (usize, usize, usize),
    pub after: (usize, usize, usize),
}

#[derive(Debug, Default)]
pub struct Diff {
    pub hunks: Vec<Hunk>,
    pub eol: Option<EolChange>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty() && self.eol.is_none()
    }
}

// ------------------------------------------------------------------- lines

/// A line's content, without its terminator, plus whether it had one.
struct Lines<'a> {
    content: Vec<&'a str>,
    /// The final line ran to the end of the text without a terminator.
    unterminated: bool,
}

fn split(text: &str) -> Lines<'_> {
    let index = LineIndex::build(text);
    let content: Vec<&str> = index
        .lines
        .iter()
        .map(|l| &text[l.start..l.content_end])
        .collect();
    let unterminated = index.lines.last().map(|l| l.end == l.content_end) == Some(true);
    Lines {
        content,
        unterminated,
    }
}

/// Report a terminator change when the *styles in use* change — a file going
/// from LF to CRLF, or an edit leaving a CRLF line in an LF file. Counts alone
/// would also fire for an ordinary added line, which the hunks already show.
fn eol_change(before: &str, after: &str) -> Option<EolChange> {
    let (_, lf_a, crlf_a, cr_a) = lines::detect_eol(before);
    let (_, lf_b, crlf_b, cr_b) = lines::detect_eol(after);
    let styles = |lf: usize, crlf: usize, cr: usize| (lf > 0, crlf > 0, cr > 0);
    if styles(lf_a, crlf_a, cr_a) == styles(lf_b, crlf_b, cr_b) {
        None
    } else {
        Some(EolChange {
            before: (lf_a, crlf_a, cr_a),
            after: (lf_b, crlf_b, cr_b),
        })
    }
}

/// Gaining or losing the final newline changes the last line even though its
/// content is identical. Terminators are otherwise not compared at all, so this
/// one case is forced by hand rather than falling through as "no change".
fn mark_final_newline(ops: &mut Vec<Op>, old: &Lines<'_>, new: &Lines<'_>) {
    if old.unterminated == new.unterminated {
        return;
    }
    // Anything other than a trailing Equal means the last line is already being
    // shown as changed.
    if ops.last() == Some(&Op::Equal) {
        *ops.last_mut().unwrap() = Op::Delete;
        ops.push(Op::Insert);
    }
}

// --------------------------------------------------------------- edit script

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    Equal,
    Delete,
    Insert,
}

// ---------------------------------------------------------------- from_edits

/// The 1-based inclusive range of old lines an edit covers. `end < start` means
/// the edit inserts between lines without removing any.
fn old_range(index: &LineIndex, text_len: usize, edit: &Edit) -> (usize, usize) {
    if edit.start < edit.end {
        let a = index.line_of_offset(edit.start);
        let b = index.line_of_offset(edit.end - 1);
        return (a, b.max(a));
    }

    // A pure insertion. Where it lands between two lines, no line is removed;
    // where it lands inside one, that line's content changes.
    if edit.start >= text_len {
        let last = index.count();
        // Appending to a file whose final line has no terminator continues that
        // line rather than starting a new one.
        let continues_last = index
            .get(last)
            .map(|l| l.end == l.content_end)
            .unwrap_or(false);
        return if continues_last {
            (last, last)
        } else {
            (last + 1, last)
        };
    }

    let a = index.line_of_offset(edit.start);
    match index.get(a) {
        Some(line) if line.start == edit.start => (a, a - 1),
        Some(_) => (a, a),
        None => (a, a.saturating_sub(1)),
    }
}

/// Hunks derived from the spans being spliced, rather than inferred by
/// comparing the two texts.
pub fn from_edits(before: &str, after: &str, edits: &[Edit], context: usize) -> Diff {
    let old = split(before);
    let new = split(after);
    let old_index = LineIndex::build(before);
    let new_index = LineIndex::build(after);

    match edit_ops(before, &old_index, &new_index, edits, &old, &new) {
        Some(mut ops) => {
            mark_final_newline(&mut ops, &old, &new);
            Diff {
                hunks: assemble(&ops, &old, &new, context),
                eol: eol_change(before, after),
            }
        }
        // The spans did not account for every line, which would mean rendering
        // a diff that does not describe the edit. Fall back rather than lie.
        None => from_texts(before, after, context),
    }
}

/// Turn the edit spans into an operation per line. Returns `None` if the result
/// does not account for exactly the lines in both texts.
fn edit_ops(
    before: &str,
    old_index: &LineIndex,
    new_index: &LineIndex,
    edits: &[Edit],
    old: &Lines<'_>,
    new: &Lines<'_>,
) -> Option<Vec<Op>> {
    let mut ops: Vec<Op> = Vec::new();
    // How far the new text has drifted from the old at a given old offset.
    let mut shift: isize = 0;
    // Next unconsumed line on each side, 1-based.
    let mut old_cursor = 1usize;
    let mut new_cursor = 1usize;

    let map = |offset: usize, shift: isize| -> usize { (offset as isize + shift).max(0) as usize };

    let mut i = 0;
    while i < edits.len() {
        let shift_before = shift;
        let (mut a, mut b) = old_range(old_index, before.len(), &edits[i]);

        // Two edits on the same line must go into one group, or the second
        // would try to remove a line the first already removed. A group that
        // removes nothing (a pure insertion) never absorbs the edit after it,
        // which stays a group of its own and lands just as cleanly.
        let mut last = i;
        let mut shift_after = shift + delta(&edits[i]);
        while last + 1 < edits.len() && b >= a {
            let (na, nb) = old_range(old_index, before.len(), &edits[last + 1]);
            if na > b {
                break;
            }
            a = a.min(na);
            b = b.max(nb);
            last += 1;
            shift_after += delta(&edits[last]);
        }
        shift = shift_after;

        // Unchanged lines ahead of this group, paired one for one.
        if a < old_cursor {
            return None;
        }
        let equal = a - old_cursor;
        ops.extend(std::iter::repeat_n(Op::Equal, equal));
        old_cursor += equal;
        new_cursor += equal;

        // Removed: the old lines the group covers.
        if b >= a {
            ops.extend(std::iter::repeat_n(Op::Delete, b - a + 1));
            old_cursor = b + 1;
        }

        // Added: the new lines occupying the same place. The group's first edit
        // starts at or after the start of line `a`, and its last ends at or
        // before the end of line `b`, so mapping those two boundaries across
        // gives the region the group produced.
        let region_start = old_index
            .get(a)
            .map(|l| l.start)
            .unwrap_or_else(|| before.len());
        let region_end = if b >= a {
            old_index
                .get(b)
                .map(|l| l.end)
                .unwrap_or_else(|| before.len())
        } else {
            region_start
        };
        let new_start_line = new_index.line_of_offset(map(region_start, shift_before));
        let new_end = map(region_end, shift_after);
        let new_end_line = if new_end > map(region_start, shift_before) {
            new_index.line_of_offset(new_end - 1)
        } else {
            new_start_line.saturating_sub(1)
        };

        if new_end_line >= new_start_line {
            if new_start_line != new_cursor {
                return None;
            }
            ops.extend(std::iter::repeat_n(
                Op::Insert,
                new_end_line - new_start_line + 1,
            ));
            new_cursor = new_end_line + 1;
        }

        i = last + 1;
    }

    // Whatever is left is unchanged on both sides.
    let old_tail = old.content.len() + 1 - old_cursor.min(old.content.len() + 1);
    let new_tail = new.content.len() + 1 - new_cursor.min(new.content.len() + 1);
    if old_tail != new_tail {
        return None;
    }
    ops.extend(std::iter::repeat_n(Op::Equal, old_tail));

    let consumed_old = ops.iter().filter(|o| **o != Op::Insert).count();
    let consumed_new = ops.iter().filter(|o| **o != Op::Delete).count();
    if consumed_old != old.content.len() || consumed_new != new.content.len() {
        return None;
    }
    Some(ops)
}

/// How many bytes an edit adds to (or removes from) the text.
fn delta(edit: &Edit) -> isize {
    edit.text.len() as isize - (edit.end - edit.start) as isize
}

// ---------------------------------------------------------------- from_texts

/// Hunks obtained by comparing two texts, for the paths with no edit list.
pub fn from_texts(before: &str, after: &str, context: usize) -> Diff {
    let old = split(before);
    let new = split(after);
    let mut ops = text_ops(&old.content, &new.content);
    mark_final_newline(&mut ops, &old, &new);
    Diff {
        hunks: assemble(&ops, &old, &new, context),
        eol: eol_change(before, after),
    }
}

/// Flatten a `similar` diff into one operation per line, which is the shape
/// [`assemble`] takes from either source.
fn text_ops(a: &[&str], b: &[&str]) -> Vec<Op> {
    let mut config = TextDiff::configure();
    config
        .algorithm(Algorithm::Myers)
        // A preview is not worth an unbounded search. Past the deadline the
        // alignment stops being minimal, which shows up as a larger hunk rather
        // than a wrong one.
        .deadline(Instant::now() + DIFF_DEADLINE);

    let mut ops = Vec::with_capacity(a.len() + b.len());
    let repeat = |ops: &mut Vec<Op>, op: Op, n: usize| ops.extend(std::iter::repeat_n(op, n));
    for op in config.diff_slices(a, b).ops() {
        match *op {
            DiffOp::Equal { len, .. } => repeat(&mut ops, Op::Equal, len),
            DiffOp::Delete { old_len, .. } => repeat(&mut ops, Op::Delete, old_len),
            DiffOp::Insert { new_len, .. } => repeat(&mut ops, Op::Insert, new_len),
            DiffOp::Replace {
                old_len, new_len, ..
            } => {
                repeat(&mut ops, Op::Delete, old_len);
                repeat(&mut ops, Op::Insert, new_len);
            }
        }
    }
    ops
}

// ----------------------------------------------------------------- assembly

/// Group an operation script into hunks, padded with `context` unchanged lines
/// and merged where two changes are close enough that the padding would meet.
fn assemble(ops: &[Op], old: &Lines<'_>, new: &Lines<'_>, context: usize) -> Vec<Hunk> {
    // Line index on each side just before operation `i`.
    let mut pos = Vec::with_capacity(ops.len() + 1);
    let (mut o, mut n) = (0usize, 0usize);
    for op in ops {
        pos.push((o, n));
        match op {
            Op::Equal => {
                o += 1;
                n += 1;
            }
            Op::Delete => o += 1,
            Op::Insert => n += 1,
        }
    }
    pos.push((o, n));

    let changes: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| **op != Op::Equal)
        .map(|(i, _)| i)
        .collect();
    if changes.is_empty() {
        return Vec::new();
    }

    let mut groups: Vec<(usize, usize)> = Vec::new();
    for &c in &changes {
        match groups.last_mut() {
            Some((_, end)) if c <= *end + 2 * context + 1 => *end = c,
            _ => groups.push((c, c)),
        }
    }

    groups
        .into_iter()
        .map(|(s, e)| {
            let hs = s.saturating_sub(context);
            let he = (e + context).min(ops.len() - 1);
            let (old_from, new_from) = pos[hs];
            let (old_to, new_to) = pos[he + 1];

            let mut rows = Vec::with_capacity(he - hs + 1);
            for (i, op) in ops.iter().enumerate().take(he + 1).skip(hs) {
                let (oi, ni) = pos[i];
                match op {
                    Op::Equal => {
                        rows.push(Row::Context(old.content[oi].to_string()));
                        mark_no_eol(&mut rows, old, oi);
                    }
                    Op::Delete => {
                        rows.push(Row::Removed(old.content[oi].to_string()));
                        mark_no_eol(&mut rows, old, oi);
                    }
                    Op::Insert => {
                        rows.push(Row::Added(new.content[ni].to_string()));
                        mark_no_eol(&mut rows, new, ni);
                    }
                }
            }

            let old_len = old_to - old_from;
            let new_len = new_to - new_from;
            Hunk {
                // A zero-length side is numbered by the line it follows, which
                // is the unified-diff convention `@@ -0,0 +1,3 @@` relies on.
                old_start: if old_len == 0 { old_from } else { old_from + 1 },
                old_len,
                new_start: if new_len == 0 { new_from } else { new_from + 1 },
                new_len,
                rows,
            }
        })
        .collect()
}

fn mark_no_eol(rows: &mut Vec<Row>, side: &Lines<'_>, index: usize) {
    if side.unterminated && index + 1 == side.content.len() {
        rows.push(Row::NoEol);
    }
}

// ---------------------------------------------------------------- rendering

/// Render as a unified diff. `max_rows` caps human output; pass `None` for the
/// complete diff.
pub fn render(diff: &Diff, old_label: &str, new_label: &str, max_rows: Option<usize>) -> String {
    let mut out = String::new();

    if let Some(change) = &diff.eol {
        out.push_str(&format!(
            "# line endings: lf={} crlf={} cr={} -> lf={} crlf={} cr={}\n",
            change.before.0,
            change.before.1,
            change.before.2,
            change.after.0,
            change.after.1,
            change.after.2
        ));
    }
    if diff.hunks.is_empty() {
        if diff.eol.is_none() {
            return out;
        }
        out.push_str("# no other textual change\n");
        return out;
    }

    out.push_str(&format!("--- {old_label}\n+++ {new_label}\n"));

    let budget = max_rows.unwrap_or(usize::MAX);
    let mut used = 0usize;
    for (i, hunk) in diff.hunks.iter().enumerate() {
        // Truncate between hunks, never inside one: half a hunk reads as the
        // whole change.
        if used > 0 && used + hunk.rows.len() > budget {
            let hunks_left = diff.hunks.len() - i;
            let rows_left: usize = diff.hunks[i..].iter().map(|h| h.rows.len()).sum();
            out.push_str(&format!(
                "# {hunks_left} more hunk(s), {rows_left} line(s), not shown; \
                 --json carries the whole diff\n"
            ));
            break;
        }
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            hunk.old_start, hunk.old_len, hunk.new_start, hunk.new_len
        ));
        for row in &hunk.rows {
            match row {
                Row::Context(line) => out.push_str(&format!(" {line}\n")),
                Row::Removed(line) => out.push_str(&format!("-{line}\n")),
                Row::Added(line) => out.push_str(&format!("+{line}\n")),
                Row::NoEol => out.push_str("\\ No newline at end of file\n"),
            }
        }
        used += hunk.rows.len();
    }
    out
}

// -------------------------------------------------------------- edit report

fn excerpt(text: &str) -> (String, bool) {
    if text.chars().count() <= MAX_EDIT_TEXT {
        return (text.to_string(), false);
    }
    (text.chars().take(MAX_EDIT_TEXT).collect(), true)
}

/// The spans an edit replaced, with their positions. This is what intact
/// actually did, as opposed to what comparing two texts suggests it did.
pub fn edits_json(before: &str, edits: &[Edit]) -> Value {
    let index = LineIndex::build(before);
    let position = |offset: usize| -> (usize, usize) {
        let line = index.line_of_offset(offset);
        let start = index.get(line).map(|l| l.start).unwrap_or(0);
        let column = before[start.min(offset)..offset].chars().count() + 1;
        (line, column)
    };

    let items: Vec<Value> = edits
        .iter()
        .take(MAX_EDITS_REPORTED)
        .map(|edit| {
            let (line, column) = position(edit.start);
            let (end_line, end_column) = position(edit.end);
            let removed = &before[edit.start..edit.end];
            let (before_text, before_cut) = excerpt(removed);
            let (after_text, after_cut) = excerpt(&edit.text);
            json!({
                "line": line,
                "column": column,
                "end_line": end_line,
                "end_column": end_column,
                "offset": edit.start,
                "end_offset": edit.end,
                "before": before_text,
                "after": after_text,
                "truncated": before_cut || after_cut,
            })
        })
        .collect();
    json!(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(before: &str, after: &str, context: usize) -> String {
        render(&from_texts(before, after, context), "a", "b", None)
    }

    fn ops_of(before: &str, after: &str) -> Vec<Op> {
        let a = split(before);
        let b = split(after);
        text_ops(&a.content, &b.content)
    }

    /// The script must consume both sides exactly, and every Equal must line up
    /// with genuinely identical content.
    fn check_script(before: &str, after: &str) {
        let a = split(before);
        let b = split(after);
        let ops = ops_of(before, after);
        let (mut i, mut j) = (0usize, 0usize);
        for op in &ops {
            match op {
                Op::Equal => {
                    assert_eq!(a.content[i], b.content[j], "misaligned Equal at {i}/{j}");
                    i += 1;
                    j += 1;
                }
                Op::Delete => i += 1,
                Op::Insert => j += 1,
            }
        }
        assert_eq!(i, a.content.len());
        assert_eq!(j, b.content.len());
    }

    #[test]
    fn scattered_changes_stay_separate_hunks() {
        let before: String = (1..=200).map(|i| format!("line {i}\n")).collect();
        let after = before
            .replace("line 5\n", "LINE FIVE\n")
            .replace("line 195\n", "LINE ONE NINE FIVE\n");
        let diff = from_texts(&before, &after, 3);
        assert_eq!(diff.hunks.len(), 2);
        let text = render(&diff, "a", "b", None);
        assert!(text.contains("-line 5\n+LINE FIVE"), "{text}");
        assert!(
            text.contains("-line 195\n+LINE ONE NINE FIVE"),
            "the second change must survive: {text}"
        );
        // Two changes, three lines of context either side, plus headers.
        assert!(text.lines().count() < 24, "{text}");
    }

    #[test]
    fn nearby_changes_merge_into_one_hunk() {
        let before = "a\nb\nc\nd\ne\nf\n";
        let after = "a\nB\nc\nd\nE\nf\n";
        assert_eq!(from_texts(before, after, 3).hunks.len(), 1);
        assert_eq!(from_texts(before, after, 0).hunks.len(), 2);
    }

    #[test]
    fn unified_header_counts_match_the_rows() {
        let text = rendered("a\nb\nc\n", "a\nx\ny\nc\n", 1);
        assert!(text.contains("--- a\n+++ b\n"), "{text}");
        assert!(text.contains("@@ -1,3 +1,4 @@"), "{text}");
    }

    #[test]
    fn insertion_only_hunk_is_numbered_like_git() {
        let text = rendered("", "hello\n", 3);
        assert!(text.contains("@@ -0,0 +1,1 @@"), "{text}");
        assert!(text.contains("+hello"), "{text}");
    }

    #[test]
    fn missing_final_newline_is_shown() {
        let text = rendered("a\nb", "a\nb\n", 3);
        assert!(
            text.contains("-b\n\\ No newline at end of file\n+b\n"),
            "{text}"
        );
        // Adding the terminator every other line already has is not a change of
        // line-ending style, so it must not be reported as one.
        assert!(!text.contains("# line endings"), "{text}");
    }

    #[test]
    fn a_stray_crlf_in_an_lf_file_is_reported() {
        let diff = from_texts("a\nb\n", "a\nb\nc\r\n", 3);
        let text = render(&diff, "a", "b", None);
        assert!(
            text.contains("# line endings: lf=2 crlf=0 cr=0 -> lf=2 crlf=1 cr=0"),
            "{text}"
        );
    }

    #[test]
    fn line_endings_are_reported_not_itemised() {
        let diff = from_texts("a\nb\nc\n", "a\r\nb\r\nc\r\n", 3);
        assert!(diff.hunks.is_empty(), "content did not change");
        let text = render(&diff, "a", "b", None);
        assert!(
            text.contains("# line endings: lf=3 crlf=0 cr=0 -> lf=0 crlf=3 cr=0"),
            "{text}"
        );
    }

    #[test]
    fn identical_texts_produce_nothing() {
        let diff = from_texts("a\nb\n", "a\nb\n", 3);
        assert!(diff.is_empty());
        assert_eq!(render(&diff, "a", "b", None), "");
    }

    #[test]
    fn alignment_holds_for_a_range_of_shapes() {
        let cases = [
            ("", ""),
            ("a\n", ""),
            ("", "a\n"),
            ("a\nb\nc\n", "a\nc\n"),
            ("a\nc\n", "a\nb\nc\n"),
            ("a\nb\nc\nd\n", "d\nc\nb\na\n"),
            ("x\nx\nx\n", "x\nx\nx\nx\n"),
            ("one\ntwo\nthree\n", "one\nTWO\nthree\n"),
            ("a\nb\na\nb\na\n", "b\na\nb\na\nb\n"),
        ];
        for (before, after) in cases {
            check_script(before, after);
        }
    }

    #[test]
    fn alignment_is_minimal() {
        // "abcabba" vs "cbabac" from Myers' paper: the shortest script is 5
        // moves, so a hunk over these should not show more.
        let a: Vec<String> = "abcabba".chars().map(|c| format!("{c}\n")).collect();
        let b: Vec<String> = "cbabac".chars().map(|c| format!("{c}\n")).collect();
        let ops = ops_of(&a.concat(), &b.concat());
        let moves = ops.iter().filter(|o| **o != Op::Equal).count();
        assert_eq!(moves, 5, "{ops:?}");
    }

    #[test]
    fn deterministic_pseudorandom_inputs_align() {
        // A tiny LCG keeps this reproducible without a dependency.
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..200 {
            let n1 = (next() % 12) as usize;
            let n2 = (next() % 12) as usize;
            let before: String = (0..n1).map(|_| format!("{}\n", next() % 6)).collect();
            let after: String = (0..n2).map(|_| format!("{}\n", next() % 6)).collect();
            check_script(&before, &after);
        }
    }

    // ---- from_edits agrees with the text diff on unambiguous edits ----

    fn edit_rendered(before: &str, edits: Vec<Edit>, context: usize) -> String {
        let after = {
            let mut out = String::new();
            let mut cursor = 0;
            for e in &edits {
                out.push_str(&before[cursor..e.start]);
                out.push_str(&e.text);
                cursor = e.end;
            }
            out.push_str(&before[cursor..]);
            out
        };
        render(&from_edits(before, &after, &edits, context), "a", "b", None)
    }

    #[test]
    fn edit_spans_produce_the_same_hunks_as_the_text_diff() {
        let before = "alpha\nbeta\ngamma\ndelta\nepsilon\n";
        // Replace inside line 2.
        let edits = vec![Edit::new(6, 10, "BETA")];
        assert_eq!(
            edit_rendered(before, edits, 1),
            rendered(before, "alpha\nBETA\ngamma\ndelta\nepsilon\n", 1)
        );
    }

    #[test]
    fn edit_spans_handle_a_pure_insertion() {
        let before = "alpha\nbeta\n";
        // Insert a whole line before line 2.
        let text = edit_rendered(before, vec![Edit::new(6, 6, "middle\n")], 3);
        assert!(text.contains("@@ -1,2 +1,3 @@"), "{text}");
        assert!(text.contains(" alpha\n+middle\n beta"), "{text}");
    }

    #[test]
    fn edit_spans_handle_a_deletion() {
        let before = "a\nb\nc\nd\n";
        let text = edit_rendered(before, vec![Edit::new(2, 6, "")], 1);
        assert!(text.contains("@@ -1,4 +1,2 @@"), "{text}");
        assert!(text.contains("-b\n-c\n"), "{text}");
    }

    #[test]
    fn edit_spans_handle_an_append() {
        let before = "a\nb\n";
        let text = edit_rendered(before, vec![Edit::new(4, 4, "c\n")], 3);
        assert!(text.contains("+c"), "{text}");
        assert!(!text.contains("-a"), "nothing was removed: {text}");
    }

    #[test]
    fn edit_spans_handle_several_scattered_edits() {
        let before: String = (1..=60).map(|i| format!("line {i}\n")).collect();
        let index = LineIndex::build(&before);
        let l5 = index.get(5).unwrap();
        let l50 = index.get(50).unwrap();
        let edits = vec![
            Edit::new(l5.start, l5.content_end, "FIVE"),
            Edit::new(l50.start, l50.content_end, "FIFTY"),
        ];
        let text = edit_rendered(&before, edits, 3);
        assert_eq!(
            text.lines().filter(|l| l.starts_with("@@")).count(),
            2,
            "{text}"
        );
        assert!(text.contains("+FIVE"), "{text}");
        assert!(text.contains("+FIFTY"), "{text}");
    }

    #[test]
    fn edit_spans_handle_a_whole_file_rewrite() {
        let before = "a\nb\nc\n";
        let text = edit_rendered(before, vec![Edit::new(0, 6, "x\n")], 3);
        assert!(text.contains("@@ -1,3 +1,1 @@"), "{text}");
    }

    #[test]
    fn edits_json_reports_positions() {
        let before = "alpha\nbeta\n";
        let value = edits_json(before, &[Edit::new(6, 10, "BETA")]);
        let item = &value[0];
        assert_eq!(item["line"], 2);
        assert_eq!(item["column"], 1);
        assert_eq!(item["before"], "beta");
        assert_eq!(item["after"], "BETA");
        assert_eq!(item["truncated"], false);
    }
}
