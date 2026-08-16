//! The editing operations. Each returns the edits to apply plus a JSON blob of
//! details for the result report; nothing here touches the filesystem.

use regex::{Regex, RegexBuilder};
use serde_json::{Value, json};

use crate::cli::{
    DeleteArgs, InsertArgs, MoveLinesArgs, ReplaceArgs, ReplaceLinesArgs, SearchArgs,
};
use crate::document::{Document, Edit};
use crate::error::{AppError, ErrorKind, Result};
use crate::lines::{self, Eol, EolMode, LineRange};

/// Settings that apply to every operation.
#[derive(Debug, Clone, Copy)]
pub struct Ctx {
    pub eol_mode: EolMode,
    pub file_eol: Eol,
}

impl Ctx {
    /// Rewrite line terminators in text that is about to be inserted.
    pub fn shape(&self, text: &str) -> String {
        match self.eol_mode.resolve(self.file_eol) {
            Some(eol) => lines::normalize_eol(text, eol),
            None => text.to_string(),
        }
    }

    pub fn eol(&self) -> &'static str {
        self.eol_mode
            .resolve(self.file_eol)
            .unwrap_or(self.file_eol)
            .as_str()
    }

    /// Shape text and make sure it ends with a line terminator.
    pub fn shape_block(&self, text: &str) -> String {
        let mut s = self.shape(text);
        if !s.is_empty() && !lines::ends_with_eol(&s) {
            // Under `--eol keep` the text keeps its own terminators, so the one
            // appended here should match them rather than the file's.
            let terminator = if self.eol_mode == EolMode::Keep && s.contains(['\n', '\r']) {
                lines::detect_eol(&s).0.as_str()
            } else {
                self.eol()
            };
            s.push_str(terminator);
        }
        s
    }
}

pub struct OpOutcome {
    pub edits: Vec<Edit>,
    pub details: Value,
    pub summary: String,
}

fn build_regex(pattern: &str, is_regex: bool, ignore_case: bool) -> Result<Regex> {
    let source = if is_regex {
        pattern.to_string()
    } else {
        regex::escape(pattern)
    };
    RegexBuilder::new(&source)
        .case_insensitive(ignore_case)
        .build()
        .map_err(|e| AppError::new(ErrorKind::Usage, format!("invalid regular expression: {e}")))
}

/// Byte range of the decoded text covered by an optional line range.
fn region(doc: &Document, range: &Option<LineRange>) -> Result<(usize, usize)> {
    match range {
        None => Ok((0, doc.text.len())),
        Some(r) => {
            let index = doc.lines();
            let (a, b) = r.resolve(index.count())?;
            let start = index.get(a).map(|l| l.start).unwrap_or(0);
            let end = index.get(b).map(|l| l.end).unwrap_or(doc.text.len());
            Ok((start, end))
        }
    }
}

fn position(doc: &Document, offset: usize) -> (usize, usize) {
    let index = doc.lines();
    let line = index.line_of_offset(offset);
    let line_start = index.get(line).map(|l| l.start).unwrap_or(0);
    let col = doc.text[line_start..offset].chars().count() + 1;
    (line, col)
}

pub fn search(doc: &Document, args: &SearchArgs, pattern: &str) -> Result<(Value, usize)> {
    let re = build_regex(pattern, args.regex, args.ignore_case)?;
    let (start, end) = region(doc, &args.lines)?;
    let hay = &doc.text[start..end];

    let mut matches = Vec::new();
    for m in re.find_iter(hay) {
        if let Some(max) = args.max {
            if matches.len() >= max {
                break;
            }
        }
        let abs = start + m.start();
        let (line, col) = position(doc, abs);
        let line_text = doc
            .lines()
            .get(line)
            .map(|l| doc.text[l.start..l.content_end].to_string())
            .unwrap_or_default();
        matches.push(json!({
            "line": line,
            "column": col,
            "offset": abs,
            "match": m.as_str(),
            "text": line_text,
        }));
    }

    let count = matches.len();
    Ok((json!({ "matches": matches, "count": count }), count))
}

pub fn replace(
    doc: &Document,
    args: &ReplaceArgs,
    find: &str,
    with: &str,
    ctx: Ctx,
) -> Result<OpOutcome> {
    if find.is_empty() {
        return Err(AppError::new(ErrorKind::Usage, "--find must not be empty"));
    }
    // The command line rules these out before parsing finishes; a batch script
    // reaches this function without passing through clap at all.
    if args.no_expand && !args.regex {
        return Err(AppError::new(
            ErrorKind::Usage,
            "no_expand applies to a regex replacement, and regex is not set",
        ));
    }
    if args.expect == Some(0) {
        return Err(AppError::new(
            ErrorKind::Usage,
            "expect 0 can never succeed: no match is exit 3, and any match is exit 4",
        ));
    }
    // A literal needle typed with \n should still match a CRLF file.
    let needle = if args.regex {
        find.to_string()
    } else {
        ctx.shape(find)
    };
    let re = build_regex(&needle, args.regex, args.ignore_case)?;
    let replacement = ctx.shape(with);

    let (start, end) = region(doc, &args.lines)?;
    let hay = &doc.text[start..end];
    let found: Vec<_> = re.captures_iter(hay).collect();
    let total = found.len();

    if total == 0 {
        return Err(AppError::new(
            ErrorKind::NoMatch,
            format!(
                "no match for {} in {}",
                describe(find, args.regex),
                doc.path.display()
            ),
        )
        .with_hint("use `intact search` to check the text, or --regex for a pattern"));
    }

    let selected: Vec<usize> = match (args.all, args.occurrence, args.expect) {
        (true, _, _) => (0..total).collect(),
        (_, Some(n), _) => {
            if n == 0 {
                return Err(AppError::new(ErrorKind::Usage, "--occurrence is 1-based"));
            }
            if n > total {
                return Err(AppError::new(
                    ErrorKind::NoMatch,
                    format!("--occurrence {n} requested but only {total} occurrence(s) found"),
                ));
            }
            vec![n - 1]
        }
        (_, _, Some(n)) => {
            if total != n {
                return Err(AppError::new(
                    ErrorKind::Ambiguous,
                    format!("--expect {n} but found {total} occurrence(s)"),
                ));
            }
            (0..total).collect()
        }
        _ => {
            if total > 1 {
                let (line, col) = position(doc, start + found[0].get(0).unwrap().start());
                return Err(AppError::new(
                    ErrorKind::Ambiguous,
                    format!(
                        "{} occurrences of {} (first at line {line}, column {col}); refusing to guess",
                        total,
                        describe(find, args.regex)
                    ),
                )
                .with_hint(
                    "pass --all to replace every occurrence, --occurrence N for one of them, \
                     --lines RANGE to narrow the region, or extend --find until it is unique",
                ));
            }
            vec![0]
        }
    };

    let expand = args.regex && !args.no_expand;
    let mut edits = Vec::with_capacity(selected.len());
    for idx in &selected {
        let caps = &found[*idx];
        let m = caps.get(0).unwrap();
        let text = if expand {
            let mut dst = String::new();
            caps.expand(&replacement, &mut dst);
            dst
        } else {
            replacement.clone()
        };
        edits.push(Edit::new(start + m.start(), start + m.end(), text));
    }

    let first = position(doc, edits[0].start);
    Ok(OpOutcome {
        summary: format!("replaced {} of {} occurrence(s)", edits.len(), total),
        details: json!({
            "occurrences_found": total,
            "occurrences_replaced": edits.len(),
            "first_line": first.0,
            "first_column": first.1,
        }),
        edits,
    })
}

pub fn insert(doc: &Document, args: &InsertArgs, text: &str, ctx: Ctx) -> Result<OpOutcome> {
    let index = doc.lines();
    let total = index.count();

    let (offset, at_line) = match (args.line, args.after) {
        (Some(spec), None) => {
            // One past the end is allowed: it means "append a new line".
            let n = spec.resolve(total, total + 1)?;
            let off = match index.get(n) {
                Some(l) => l.start,
                None => doc.text.len(),
            };
            (off, n)
        }
        (None, Some(spec)) => {
            let n = spec.resolve(total, total)?;
            let off = index.get(n).map(|l| l.end).ok_or_else(|| {
                AppError::new(ErrorKind::Range, format!("line {n} does not exist"))
            })?;
            (off, n + 1)
        }
        (None, None) => {
            return Err(AppError::new(
                ErrorKind::Usage,
                "insert requires --line N or --after N",
            ));
        }
        (Some(_), Some(_)) => {
            return Err(AppError::new(
                ErrorKind::Usage,
                "--line and --after are mutually exclusive",
            ));
        }
    };

    let mut block = ctx.shape_block(text);
    // Inserting at the very end of a file whose last line has no terminator
    // must start a new line first.
    if offset == doc.text.len() && !doc.text.is_empty() && !lines::ends_with_eol(&doc.text) {
        block.insert_str(0, ctx.eol());
    }
    let inserted_lines = count_lines(&block);

    Ok(OpOutcome {
        summary: format!("inserted {inserted_lines} line(s) at line {at_line}"),
        details: json!({ "at_line": at_line, "lines_inserted": inserted_lines }),
        edits: vec![Edit::new(offset, offset, block)],
    })
}

pub fn append(doc: &Document, text: &str, ctx: Ctx, trailing_newline: bool) -> Result<OpOutcome> {
    let mut block = if trailing_newline {
        ctx.shape_block(text)
    } else {
        ctx.shape(text)
    };
    if !doc.text.is_empty() && !lines::ends_with_eol(&doc.text) {
        block.insert_str(0, ctx.eol());
    }
    let offset = doc.text.len();
    Ok(OpOutcome {
        summary: format!("appended {} character(s)", block.chars().count()),
        details: json!({ "at_line": doc.lines().count() + 1 }),
        edits: vec![Edit::new(offset, offset, block)],
    })
}

pub fn prepend(doc: &Document, text: &str, ctx: Ctx, trailing_newline: bool) -> Result<OpOutcome> {
    let block = if trailing_newline && !doc.text.is_empty() {
        ctx.shape_block(text)
    } else {
        ctx.shape(text)
    };
    Ok(OpOutcome {
        summary: format!("prepended {} character(s)", block.chars().count()),
        details: json!({ "at_line": 1 }),
        edits: vec![Edit::new(0, 0, block)],
    })
}

pub fn delete(doc: &Document, args: &DeleteArgs) -> Result<OpOutcome> {
    let index = doc.lines();
    let (a, b) = args.lines.resolve(index.count())?;
    let start = index.get(a).unwrap().start;
    let end = index.get(b).unwrap().end;
    Ok(OpOutcome {
        summary: format!("deleted line(s) {a}:{b}"),
        details: json!({ "from_line": a, "to_line": b, "lines_deleted": b - a + 1 }),
        edits: vec![Edit::new(start, end, String::new())],
    })
}

pub fn replace_lines(
    doc: &Document,
    args: &ReplaceLinesArgs,
    text: &str,
    ctx: Ctx,
) -> Result<OpOutcome> {
    let index = doc.lines();
    let (a, b) = args.lines.resolve(index.count())?;
    let start = index.get(a).unwrap().start;
    let last = index.get(b).unwrap();
    let end = last.end;
    // If the replaced region did not end with a terminator (last line of a file
    // without a trailing newline), the replacement should not gain one.
    let region_had_eol = last.end > last.content_end;
    let block = if region_had_eol {
        ctx.shape_block(text)
    } else {
        ctx.shape(text)
    };

    let new_lines = count_lines(&block);
    Ok(OpOutcome {
        summary: format!("replaced line(s) {a}:{b} with {new_lines} line(s)"),
        details: json!({ "from_line": a, "to_line": b, "lines_replaced": b - a + 1, "lines_written": new_lines }),
        edits: vec![Edit::new(start, end, block)],
    })
}

/// Move a block of lines somewhere else in the same file.
///
/// The destination is named in the file's *current* numbering — the numbers
/// `view --number` prints — rather than the numbering the file is left with
/// once the block has been lifted out of it. The alternative asks the caller to
/// do that renumbering in their head before naming a line they can see, and to
/// get a different answer depending on whether the block moves up or down.
///
/// The moved lines keep their own terminators. Nothing here is new text: the
/// file's bytes are being permuted, so rewriting their line endings on the way
/// past (what [`Ctx::shape`] would do) would change lines the caller only asked
/// to relocate. The one terminator this can add — to a block lifted from an
/// unterminated end of file — does go through `--eol`.
pub fn move_lines(doc: &Document, args: &MoveLinesArgs, ctx: Ctx) -> Result<OpOutcome> {
    let index = doc.lines();
    let total = index.count();
    let (a, b) = args.lines.resolve(total)?;
    let count = b - a + 1;

    if a == 1 && b == total {
        return Err(AppError::new(
            ErrorKind::Usage,
            format!("lines {a}:{b} are the whole file; there is nowhere to move them"),
        ));
    }

    // Every destination form collapses to an insertion gap: `gap` means "before
    // line gap", running from 1 (the top of the file) to total + 1 (the end of
    // it). Reducing the three of them to one number is what makes them
    // comparable against the block below.
    let gap = match (args.after, args.before, args.by) {
        (Some(spec), None, None) => spec.resolve(total, total)? + 1,
        // One past the last line is accepted here as it is for `insert --line`,
        // and means the end of the file.
        (None, Some(spec), None) => spec.resolve(total, total + 1)?,
        (None, None, Some(k)) => shift_gap(k, a, b, total)?,
        (None, None, None) => {
            return Err(AppError::new(
                ErrorKind::Usage,
                "move-lines requires --after N, --before N or --by K",
            ));
        }
        _ => {
            return Err(AppError::new(
                ErrorKind::Usage,
                "--after, --before and --by are mutually exclusive",
            ));
        }
    };

    if gap > a && gap <= b {
        return Err(AppError::new(
            ErrorKind::Usage,
            format!("the destination is inside lines {a}:{b}, the block being moved"),
        )
        .with_hint(format!(
            "name a line outside {a}:{b} — the block cannot be moved into itself"
        )));
    }

    let first = index.get(a).unwrap();
    let last = index.get(b).unwrap();
    let (block_start, block_end) = (first.start, last.end);

    if gap == a || gap == b + 1 {
        // The block is already there. Not an error: nothing about the request
        // was ambiguous, and a script that computes a destination should be
        // able to arrive at the current one without failing.
        return Ok(OpOutcome {
            summary: format!("line(s) {a}:{b} are already there; nothing moved"),
            details: json!({
                "from_line": a, "to_line": b, "lines_moved": 0, "at_line": a, "at_end_line": b,
            }),
            edits: Vec::new(),
        });
    }

    let dest = match index.get(gap) {
        Some(line) => line.start,
        None => doc.text.len(),
    };

    let mut delete_from = block_start;
    let mut block = doc.text[block_start..block_end].to_string();

    // Two ends of one rule: the file keeps its final newline, or its lack of
    // one, exactly as it had it.
    if last.end == last.content_end {
        // The block is the file's unterminated tail. Lifting it out would leave
        // the line above it as a terminated last line, so that terminator goes
        // with the block; the block gains one of its own, its destination being
        // somewhere in the middle of the file.
        //
        // `a > 1` holds here: a block reaching line 1 as well as the last line
        // is the whole file, refused above.
        delete_from = index.get(a - 1).unwrap().content_end;
        block.push_str(ctx.eol());
    } else if dest == doc.text.len() && !lines::ends_with_eol(&doc.text) {
        // Landing after a last line that has no terminator: it needs one before
        // the block can follow it, and the block gives up its own so that the
        // file still ends without one.
        block.truncate(last.content_end - block_start);
        block.insert_str(0, ctx.eol());
    }

    // Where the block ends up once the lines it was lifted from have closed up
    // behind it. Moving up, the gap keeps its number; moving down, everything
    // from the gap back to the block has shifted up by the block's length.
    let at_line = if gap < a { gap } else { gap - count };

    Ok(OpOutcome {
        summary: format!(
            "moved {count} line(s) from {a}:{b} to {at_line}:{}",
            at_line + count - 1
        ),
        details: json!({
            "from_line": a,
            "to_line": b,
            "lines_moved": count,
            "at_line": at_line,
            "at_end_line": at_line + count - 1,
        }),
        // Disjoint by construction: a destination between the two ends of the
        // block was refused above, so `validate_edits` sorts these into a clean
        // lift-and-drop whichever way the block travels.
        edits: vec![
            Edit::new(dest, dest, block),
            Edit::new(delete_from, block_end, String::new()),
        ],
    })
}

/// `--by K` as an insertion gap. K counts lines in the file as it stands: the
/// block's first line ends up at `a + K` once everything has closed up, which
/// is the same number whichever direction it travelled.
fn shift_gap(k: i64, a: usize, b: usize, total: usize) -> Result<usize> {
    if k == 0 {
        return Err(AppError::new(
            ErrorKind::Usage,
            "--by 0 moves the block nowhere",
        ));
    }
    if k > 0 {
        let step = k as u64;
        if b as u64 + step > total as u64 {
            return Err(AppError::new(
                ErrorKind::Range,
                format!(
                    "moving line(s) {a}:{b} down by {step} would run past the end of a \
                     {total}-line file"
                ),
            )
            .with_hint(format!(
                "the furthest down they can go is --by {}",
                total - b
            )));
        }
        Ok(b + 1 + step as usize)
    } else {
        let step = k.unsigned_abs();
        if step >= a as u64 {
            return Err(AppError::new(
                ErrorKind::Range,
                format!("moving line(s) {a}:{b} up by {step} would run past the start of the file"),
            )
            .with_hint(format!("the furthest up they can go is --by -{}", a - 1)));
        }
        Ok(a - step as usize)
    }
}

pub fn write_all(
    doc: &Document,
    text: &str,
    ctx: Ctx,
    trailing_newline: bool,
) -> Result<OpOutcome> {
    let block = if trailing_newline {
        ctx.shape_block(text)
    } else {
        ctx.shape(text)
    };
    let new_lines = count_lines(&block);
    Ok(OpOutcome {
        summary: format!("wrote {new_lines} line(s)"),
        details: json!({ "lines_written": new_lines }),
        edits: vec![Edit::new(0, doc.text.len(), block)],
    })
}

fn count_lines(text: &str) -> usize {
    lines::LineIndex::build(text).count()
}

fn describe(find: &str, is_regex: bool) -> String {
    let kind = if is_regex { "pattern" } else { "text" };
    let shown: String = find.chars().take(60).collect();
    let ellipsis = if find.chars().count() > 60 { "…" } else { "" };
    format!("{kind} {shown:?}{ellipsis}")
}
