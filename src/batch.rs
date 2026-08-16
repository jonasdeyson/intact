//! `intact batch`: a JSON script of operations, applied across one or more
//! files as a single transaction — nothing is written unless every operation
//! in the script succeeds.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::cli::{self, Cli};
use crate::diff;
use crate::document::{Document, ForcedEncoding};
use crate::error::{AppError, ErrorKind, Result};
use crate::lines::{self, LineRange};
use crate::ops::{self, Ctx, OpOutcome};
use crate::policy::{binary_policy, ctx_for, preflight};
use crate::report::{Report, emit_diff, wants_diff};
use crate::textsrc;

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum Script {
    Wrapped { ops: Vec<BatchOp> },
    Bare(Vec<BatchOp>),
}

/// Every variant carries an optional `file`, which is what makes one script
/// able to edit several files. `deny_unknown_fields` still holds, so a typo in
/// any field name fails loudly; that rules out `#[serde(flatten)]`, which serde
/// cannot combine with it, hence the field being repeated per variant.
#[derive(serde::Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
enum BatchOp {
    Replace {
        #[serde(default)]
        file: Option<PathBuf>,
        find: String,
        #[serde(default)]
        with: String,
        #[serde(default)]
        regex: bool,
        #[serde(default)]
        ignore_case: bool,
        #[serde(default)]
        all: bool,
        #[serde(default)]
        occurrence: Option<usize>,
        #[serde(default)]
        expect: Option<usize>,
        #[serde(default)]
        lines: Option<Value>,
        #[serde(default)]
        no_expand: bool,
    },
    Insert {
        #[serde(default)]
        file: Option<PathBuf>,
        #[serde(default)]
        line: Option<Value>,
        #[serde(default)]
        after: Option<Value>,
        text: String,
    },
    Append {
        #[serde(default)]
        file: Option<PathBuf>,
        text: String,
    },
    Prepend {
        #[serde(default)]
        file: Option<PathBuf>,
        text: String,
    },
    Delete {
        #[serde(default)]
        file: Option<PathBuf>,
        lines: Value,
    },
    #[serde(rename = "replace-lines")]
    ReplaceLines {
        #[serde(default)]
        file: Option<PathBuf>,
        lines: Value,
        text: String,
    },
    #[serde(rename = "move-lines")]
    MoveLines {
        #[serde(default)]
        file: Option<PathBuf>,
        lines: Value,
        #[serde(default)]
        after: Option<Value>,
        #[serde(default)]
        before: Option<Value>,
        #[serde(default)]
        by: Option<i64>,
    },
    Write {
        #[serde(default)]
        file: Option<PathBuf>,
        text: String,
    },
}

impl BatchOp {
    fn file(&self) -> Option<&Path> {
        let file = match self {
            BatchOp::Replace { file, .. }
            | BatchOp::Insert { file, .. }
            | BatchOp::Append { file, .. }
            | BatchOp::Prepend { file, .. }
            | BatchOp::Delete { file, .. }
            | BatchOp::ReplaceLines { file, .. }
            | BatchOp::MoveLines { file, .. }
            | BatchOp::Write { file, .. } => file,
        };
        file.as_deref()
    }
}

fn as_range(value: &Value) -> Result<LineRange> {
    let s = match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        other => {
            return Err(AppError::new(
                ErrorKind::Usage,
                format!("line range must be a string or number, got {other}"),
            ));
        }
    };
    s.parse::<LineRange>()
        .map_err(|e| AppError::new(ErrorKind::Usage, e))
}

fn as_spec(value: &Value) -> Result<lines::LineSpec> {
    let s = match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        other => {
            return Err(AppError::new(
                ErrorKind::Usage,
                format!("line must be a string or number, got {other}"),
            ));
        }
    };
    s.parse::<lines::LineSpec>()
        .map_err(|e| AppError::new(ErrorKind::Usage, e))
}

pub fn run(cli: &Cli, args: &cli::BatchArgs, forced: Option<ForcedEncoding>) -> Result<i32> {
    let raw = if args.script == Path::new("-") {
        textsrc::read_stdin()?
    } else {
        textsrc::read_utf8_file(&args.script)?
    };
    let script: Script = serde_json::from_str(&raw)
        .map_err(|e| AppError::new(ErrorKind::Usage, format!("invalid batch script: {e}")))?;
    let batch_ops = match script {
        Script::Wrapped { ops } => ops,
        Script::Bare(ops) => ops,
    };
    if batch_ops.is_empty() {
        return Err(AppError::new(
            ErrorKind::Usage,
            "batch script contains no operations",
        ));
    }

    // One entry per file the script touches, in the order it was first named,
    // so a later operation on the same file sees the earlier one's result.
    let mut targets: Vec<BatchTarget> = Vec::new();

    for (i, op) in batch_ops.iter().enumerate() {
        let path = match (op.file(), &args.file) {
            (Some(p), _) => p.to_path_buf(),
            (None, Some(default)) => default.clone(),
            (None, None) => {
                return Err(AppError::new(
                    ErrorKind::Usage,
                    format!(
                        "operation {}: no file to edit — give the op a \"file\", \
                         or name a default file on the command line",
                        i + 1
                    ),
                )
                .with_hint("intact batch FILE --script ..., or {\"op\":...,\"file\":\"path\"}"));
            }
        };

        let slot = match targets.iter().position(|t| t.original.path == path) {
            Some(idx) => idx,
            None => {
                let original = Document::load(&path, forced, binary_policy(cli))?;
                preflight(cli, &original)?;
                // Pin the encoding so re-decoding between steps cannot drift,
                // keeping the original source so `detected_by` stays truthful.
                let pinned = Some(ForcedEncoding {
                    encoding: original.encoding,
                    source: original.detection,
                });
                let current = original.raw.clone();
                targets.push(BatchTarget {
                    original,
                    pinned,
                    current,
                    ops: 0,
                });
                targets.len() - 1
            }
        };

        let target = &mut targets[slot];
        let doc = Document::from_bytes(
            path.clone(),
            std::mem::take(&mut target.current),
            target.pinned,
            true,
        );
        let ctx = ctx_for(cli, &doc);
        let outcome = apply_batch_op(&doc, op, ctx).map_err(|e| AppError {
            message: format!("operation {} ({}): {}", i + 1, path.display(), e.message),
            ..e
        })?;
        let mut edits = outcome.edits;
        target.current = doc.build_output(&mut edits, cli.unmappable, cli.lossy)?;
        target.ops += 1;
    }

    // Nothing is written until every operation across every file has succeeded,
    // so a failing operation leaves all of them untouched.
    let mut file_reports = Vec::with_capacity(targets.len());
    let mut any_changed = false;

    for target in &targets {
        let changed = target.current != target.original.raw;
        any_changed |= changed;
        let final_doc = Document::from_bytes(
            target.original.path.clone(),
            target.current.clone(),
            target.pinned,
            true,
        );

        let mut report = Report::new("batch", &target.original);
        report.summary = format!("applied {} operation(s)", target.ops);
        report.changed = changed;
        report.dry_run = cli.dry_run;
        report.bytes_after = target.current.len();
        report.lines_after = final_doc.lines().count();
        report.details = json!({ "operations": target.ops });

        // Each step re-edits the result of the last, so the spans of any one of
        // them describe a document that no longer exists. Comparing the two ends
        // is the only honest account of a batch.
        if wants_diff(cli) {
            let diff = diff::from_texts(&target.original.text, &final_doc.text, cli.diff_context);
            emit_diff(cli, &mut report, &diff, true, changed);
        }
        file_reports.push(report);
    }

    if !cli.dry_run {
        for target in &targets {
            if target.current != target.original.raw {
                target.original.save(&target.current, cli.backup)?;
            }
        }
    }

    // A `files` array whatever the count: a shape that changed with the number
    // of files would be one more thing for a caller to branch on.
    if cli.json {
        println!(
            "{}",
            json!({
                "ok": true,
                "command": "batch",
                "operations": batch_ops.len(),
                "changed": any_changed,
                "dry_run": cli.dry_run,
                "files": file_reports.iter().map(Report::to_file_json).collect::<Vec<_>>(),
            })
        );
    } else {
        // `to_file_json` carries the warnings in JSON mode; this is the human
        // equivalent, on stderr and not gated by --quiet like print_report.
        for report in &file_reports {
            for warning in &report.warnings {
                eprintln!("intact: warning: {}: {warning}", report.path);
            }
        }
        if !cli.quiet {
            for report in &file_reports {
                println!("{}", report.human());
            }
        }
    }
    Ok(0)
}

/// One file a batch script touches, carried across the operations that name it.
struct BatchTarget {
    original: Document,
    pinned: Option<ForcedEncoding>,
    /// The file's bytes as of the last applied operation.
    current: Vec<u8>,
    ops: usize,
}

/// The op's `file` has already been resolved into `doc` by the caller, so it is
/// ignored here.
fn apply_batch_op(doc: &Document, op: &BatchOp, ctx: Ctx) -> Result<OpOutcome> {
    // Text in a script never goes through --escapes: JSON has its own escapes.
    let no_text = cli::TextSource {
        text: None,
        text_file: None,
    };
    match op {
        BatchOp::Replace {
            file: _,
            find,
            with,
            regex,
            ignore_case,
            all,
            occurrence,
            expect,
            lines: range,
            no_expand,
        } => {
            let args = cli::ReplaceArgs {
                file: doc.path.clone(),
                find: None,
                find_file: None,
                with: None,
                with_file: None,
                delete: false,
                regex: *regex,
                ignore_case: *ignore_case,
                all: *all,
                occurrence: *occurrence,
                expect: *expect,
                lines: range.as_ref().map(as_range).transpose()?,
                no_expand: *no_expand,
            };
            ops::replace(doc, &args, find, with, ctx)
        }
        BatchOp::Insert {
            file: _,
            line,
            after,
            text,
        } => {
            let args = cli::InsertArgs {
                file: doc.path.clone(),
                line: line.as_ref().map(as_spec).transpose()?,
                after: after.as_ref().map(as_spec).transpose()?,
                text: no_text,
            };
            ops::insert(doc, &args, text, ctx)
        }
        BatchOp::Append { file: _, text } => ops::append(doc, text, ctx, true),
        BatchOp::Prepend { file: _, text } => ops::prepend(doc, text, ctx, true),
        BatchOp::Delete {
            file: _,
            lines: range,
        } => {
            let args = cli::DeleteArgs {
                file: doc.path.clone(),
                lines: as_range(range)?,
            };
            ops::delete(doc, &args)
        }
        BatchOp::ReplaceLines {
            file: _,
            lines: range,
            text,
        } => {
            let args = cli::ReplaceLinesArgs {
                file: doc.path.clone(),
                lines: as_range(range)?,
                text: no_text,
            };
            ops::replace_lines(doc, &args, text, ctx)
        }
        BatchOp::MoveLines {
            file: _,
            lines: range,
            after,
            before,
            by,
        } => {
            let args = cli::MoveLinesArgs {
                file: doc.path.clone(),
                lines: as_range(range)?,
                after: after.as_ref().map(as_spec).transpose()?,
                before: before.as_ref().map(as_spec).transpose()?,
                by: *by,
            };
            ops::move_lines(doc, &args, ctx)
        }
        BatchOp::Write { file: _, text } => ops::write_all(doc, text, ctx, true),
    }
}
