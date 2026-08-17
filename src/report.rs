//! Result reporting in both human and JSON form, and the shared tail every
//! writing command ends in: build the new bytes, diff them, save, report.

use serde_json::{Map, Value, json};

use crate::cli::Cli;
use crate::diff;
use crate::document::Document;
use crate::error::{AppError, Result};
use crate::lines::{self, LineIndex};
use crate::ops::OpOutcome;

pub struct Report {
    pub command: &'static str,
    pub path: String,
    /// The file `path` points at, when it is a symlink. Reported so that an
    /// edit landing somewhere other than the named path is visible.
    pub resolved_path: Option<String>,
    pub encoding: String,
    pub detected_by: &'static str,
    pub bom: bool,
    pub eol: &'static str,
    pub changed: bool,
    pub dry_run: bool,
    pub bytes_before: usize,
    pub bytes_after: usize,
    pub lines_before: usize,
    pub lines_after: usize,
    pub summary: String,
    pub details: Value,
    /// Advisories that do not block the write. Collected here rather than at
    /// each call site because `Report::new` is the one point every write path
    /// (`finish`, `convert`, `batch`) passes through.
    pub warnings: Vec<String>,
}

impl Report {
    pub fn new(command: &'static str, doc: &Document) -> Report {
        Report {
            command,
            path: doc.path.display().to_string(),
            resolved_path: crate::atomic::link_target(&doc.path).map(|p| p.display().to_string()),
            encoding: doc.encoding.name().to_string(),
            detected_by: doc.detection.as_str(),
            bom: doc.bom.is_some(),
            eol: doc.eol.name(),
            changed: false,
            dry_run: false,
            bytes_before: doc.raw.len(),
            bytes_after: doc.raw.len(),
            lines_before: doc.lines().count(),
            lines_after: doc.lines().count(),
            summary: String::new(),
            details: Value::Null,
            warnings: doc.mojibake_warning().into_iter().collect(),
        }
    }

    pub fn to_json(&self) -> Value {
        let mut obj = Map::new();
        obj.insert("ok".into(), json!(true));
        obj.insert("command".into(), json!(self.command));
        self.write_fields(&mut obj);
        Value::Object(obj)
    }

    /// The same fields without `ok`/`command`, for one entry of `batch`'s
    /// `files` array, whose owning object already carries those.
    pub fn to_file_json(&self) -> Value {
        let mut obj = Map::new();
        self.write_fields(&mut obj);
        Value::Object(obj)
    }

    /// Written once so the two JSON shapes above cannot drift apart.
    fn write_fields(&self, obj: &mut Map<String, Value>) {
        obj.insert("path".into(), json!(self.path));
        // Only present for a symlink, so its presence is itself the signal.
        if let Some(resolved) = &self.resolved_path {
            obj.insert("resolved_path".into(), json!(resolved));
        }
        obj.insert("encoding".into(), json!(self.encoding));
        obj.insert("detected_by".into(), json!(self.detected_by));
        obj.insert("bom".into(), json!(self.bom));
        obj.insert("eol".into(), json!(self.eol));
        obj.insert("changed".into(), json!(self.changed));
        obj.insert("dry_run".into(), json!(self.dry_run));
        obj.insert("bytes_before".into(), json!(self.bytes_before));
        obj.insert("bytes_after".into(), json!(self.bytes_after));
        obj.insert("lines_before".into(), json!(self.lines_before));
        obj.insert("lines_after".into(), json!(self.lines_after));
        obj.insert("summary".into(), json!(self.summary));
        // Absent rather than empty, so its presence is the signal - as with
        // `resolved_path` above.
        if !self.warnings.is_empty() {
            obj.insert("warnings".into(), json!(self.warnings));
        }
        if let Value::Object(details) = &self.details {
            for (k, v) in details {
                obj.insert(k.clone(), v.clone());
            }
        }
    }

    pub fn human(&self) -> String {
        let verb = if self.dry_run {
            "would change"
        } else if self.changed {
            "updated"
        } else {
            "unchanged"
        };
        let path = match &self.resolved_path {
            Some(resolved) => format!("{} -> {}", self.path, resolved),
            None => self.path.clone(),
        };
        format!(
            "{}: {} ({}, {}{}) - {}",
            path,
            verb,
            self.encoding,
            self.eol,
            if self.bom { ", bom" } else { "" },
            self.summary
        )
    }
}

pub fn print_report(report: &Report, json_mode: bool, quiet: bool) {
    if json_mode {
        println!(
            "{}",
            serde_json::to_string(&report.to_json()).unwrap_or_default()
        );
    } else {
        // Warnings go to stderr and ignore --quiet: --quiet suppresses the
        // routine success line, not a caution about the file's encoding.
        for warning in &report.warnings {
            eprintln!("intact: warning: {}: {warning}", report.path);
        }
        if !quiet {
            println!("{}", report.human());
        }
    }
}

pub fn print_error(err: &AppError, json_mode: bool) {
    if json_mode {
        let value = json!({
            "ok": false,
            "error": err.message,
            "kind": err.kind.as_str(),
            "hint": err.hint,
            "exit_code": err.kind.exit_code(),
        });
        eprintln!("{}", serde_json::to_string(&value).unwrap_or_default());
    } else {
        eprintln!("intact: {err}");
    }
}

// ------------------------------------------------- the tail of every write

/// A diff is produced for `--dry-run` (where it is the whole point) and for
/// `--show-diff`, which reports an edit that was actually applied. The latter
/// is what puts the change in front of a human without a second command and a
/// second approval.
pub fn wants_diff(cli: &Cli) -> bool {
    cli.dry_run || cli.show_diff
}

pub fn merge_details(report: &mut Report, extra: Map<String, Value>) {
    match &mut report.details {
        Value::Object(map) => map.extend(extra),
        other => *other = Value::Object(extra),
    }
}

/// Print the diff above the summary line, or attach it to the JSON result.
/// Human output is capped; JSON is not, since it is not being read by eye.
pub fn emit_diff(cli: &Cli, report: &mut Report, diff: &diff::Diff, existed: bool, changed: bool) {
    // --quiet suppresses the summary line; a diff that was explicitly asked for
    // is not that, and suppressing it would leave `--show-diff -q` — the way to
    // get a patch on stdout and nothing else — with no output at all.

    // "would change" with nothing shown is the one outcome a preview must never
    // produce, so an invisible change is spelled out rather than left blank.
    if diff.is_empty() {
        if changed && !cli.json {
            println!("# no textual change; the bytes differ (encoding or byte-order mark)");
        }
        return;
    }

    let new_label = report.path.clone();
    let old_label = if existed {
        new_label.clone()
    } else {
        "/dev/null".to_string()
    };

    if cli.json {
        let mut extra = Map::new();
        extra.insert(
            "diff".into(),
            json!(diff::render(diff, &old_label, &new_label, None)),
        );
        if let Some(change) = &diff.eol {
            let counts =
                |(lf, crlf, cr): (usize, usize, usize)| json!({ "lf": lf, "crlf": crlf, "cr": cr });
            extra.insert("eol_before".into(), counts(change.before));
            extra.insert("eol_after".into(), counts(change.after));
        }
        merge_details(report, extra);
    } else {
        print!(
            "{}",
            diff::render(diff, &old_label, &new_label, Some(diff::MAX_RENDERED_ROWS))
        );
    }
}

pub fn finish(cli: &Cli, doc: &Document, command: &'static str, outcome: OpOutcome) -> Result<i32> {
    let OpOutcome {
        mut edits,
        details,
        summary,
    } = outcome;
    // Before anything is built: under --no-guess this is a refusal, and a
    // refusal that reported a diff first would be reporting a write that is not
    // going to happen. --dry-run is included deliberately, for the same reason
    // `preflight` refuses there — a preview of a write the flags forbid is not
    // a preview of anything.
    let undeclared = doc.undeclared_ascii_write(&edits);
    if let (Some(ch), true) = (undeclared, cli.no_guess) {
        return Err(doc.undeclared_ascii_error(ch));
    }
    // build_output sorts the edits, which apply_to_text and the diff both rely
    // on to walk the text in one pass.
    let bytes = doc.build_output(&mut edits, cli.unmappable, cli.lossy)?;
    let new_text = doc.apply_to_text(&edits);
    // A brand-new file must be created even when its content is empty, so
    // "nothing to write" is not the same question as "bytes are unchanged".
    let changed = bytes != doc.raw || !doc.existed;

    let mut report = Report::new(command, doc);
    if let Some(ch) = undeclared {
        report.warnings.push(doc.undeclared_ascii_warning(ch));
    }
    report.summary = summary;
    report.details = details;
    report.changed = changed;
    report.dry_run = cli.dry_run;
    report.bytes_after = bytes.len();
    report.lines_after = LineIndex::build(&new_text).count();
    // Report the line endings the file ends up with, not the ones it had: for a
    // new file those differ whenever --eol was used.
    report.eol = lines::detect_eol(&new_text).0.name();

    // The spans are what intact actually replaced. Reporting them beats any
    // diff of the two texts, which can only infer that after the fact.
    if cli.json {
        let mut extra = Map::new();
        extra.insert("edits".into(), diff::edits_json(&doc.text, &edits));
        extra.insert("edit_count".into(), json!(edits.len()));
        merge_details(&mut report, extra);
    }

    if wants_diff(cli) {
        let diff = diff::from_edits(&doc.text, &new_text, &edits, cli.diff_context);
        emit_diff(cli, &mut report, &diff, doc.existed, changed);
    }

    if !cli.dry_run && changed {
        doc.save(&bytes, cli.backup)?;
    }

    print_report(&report, cli.json, cli.quiet);
    Ok(0)
}
