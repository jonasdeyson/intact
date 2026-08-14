//! Result reporting in both human and JSON form.

use serde_json::{json, Map, Value};

use crate::document::Document;
use crate::error::AppError;

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
}

impl Report {
    pub fn new(command: &'static str, doc: &Document) -> Report {
        Report {
            command,
            path: doc.path.display().to_string(),
            resolved_path: crate::document::link_target(&doc.path).map(|p| p.display().to_string()),
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
    } else if !quiet {
        println!("{}", report.human());
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
