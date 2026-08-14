mod cli;
mod diff;
mod document;
mod encoding_util;
mod error;
mod lines;
mod manual;
mod ops;
mod report;
mod textsrc;

use std::path::Path;

use clap::Parser;
use serde_json::{json, Value};

use cli::{BomMode, Cli, Command};
use document::{Detection, Document, ForcedEncoding};
use encoding_util::{encode_text, BomKind};
use error::{AppError, ErrorKind, Result};
use lines::{Eol, EolMode, LineIndex, LineRange};
use ops::{Ctx, OpOutcome};
use report::Report;

/// Default encoding for every invocation, overridden by --encoding.
const ENV_ENCODING: &str = "INTACT_ENCODING";
/// Set to 1/true/yes to refuse writing to a file whose encoding was guessed.
const ENV_NO_GUESS: &str = "INTACT_NO_GUESS";
/// Line endings for every invocation, overridden by --eol.
const ENV_EOL: &str = "INTACT_EOL";
/// Set to 1/true/yes to refuse writing to a file whose line endings differ.
const ENV_STRICT_EOL: &str = "INTACT_STRICT_EOL";

fn main() {
    let cli = Cli::parse();
    let json_mode = cli.json;
    match run(&cli) {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            report::print_error(&err, json_mode);
            std::process::exit(err.kind.exit_code());
        }
    }
}

/// `--encoding` wins; otherwise INTACT_ENCODING, so a project that mandates
/// one encoding can set it once instead of relying on every invocation to
/// remember the flag.
fn resolve_forced_encoding(cli: &Cli) -> Result<Option<ForcedEncoding>> {
    if let Some(label) = &cli.encoding {
        return Ok(Some(ForcedEncoding::flag(
            encoding_util::encoding_for_label(label)?,
        )));
    }
    match std::env::var(ENV_ENCODING) {
        Ok(label) if !label.trim().is_empty() => {
            let encoding = encoding_util::encoding_for_label(&label)
                .map_err(|e| AppError::new(e.kind, format!("{ENV_ENCODING}: {}", e.message)))?;
            Ok(Some(ForcedEncoding::environment(encoding)))
        }
        _ => Ok(None),
    }
}

/// `--eol` wins; otherwise INTACT_EOL; otherwise match the file.
fn resolve_eol_mode(cli: &Cli) -> Result<EolMode> {
    if let Some(mode) = cli.eol {
        return Ok(mode);
    }
    match std::env::var(ENV_EOL) {
        Ok(value) if !value.trim().is_empty() => {
            <EolMode as clap::ValueEnum>::from_str(value.trim(), true).map_err(|_| {
                AppError::new(
                    ErrorKind::Usage,
                    format!("{ENV_EOL}: unknown line-ending mode '{value}'"),
                )
                .with_hint("expected one of: auto, lf, crlf, cr, keep")
            })
        }
        _ => Ok(EolMode::Auto),
    }
}

fn run(cli: &Cli) -> Result<i32> {
    let forced = resolve_forced_encoding(cli)?;
    // Validate up front so a typo in the environment fails on every command,
    // rather than only on the ones that happen to consult it.
    resolve_eol_mode(cli)?;

    match &cli.command {
        Command::Encodings => {
            cmd_encodings(cli);
            Ok(0)
        }
        Command::Guide(args) => cmd_guide(cli, args),
        Command::Instructions(args) => {
            // The global --encoding and --eol double as "this project mandates
            // X"; they arrive here already validated and canonicalised.
            let eol_mode = resolve_eol_mode(cli)?;
            let eol = match eol_mode {
                EolMode::Lf => Some("lf"),
                EolMode::Crlf => Some("crlf"),
                EolMode::Cr => Some("cr"),
                EolMode::Auto | EolMode::Keep => None,
            };
            // --wsl names the launcher the agent has to put in front of every
            // command; --wsl DISTRO pins which distribution it runs in.
            let launcher = args.wsl.as_ref().map(|distro| match distro {
                Some(name) => format!("wsl.exe -d {name}"),
                None => "wsl.exe".to_string(),
            });
            print!(
                "{}",
                manual::instructions(&manual::InstructionsSpec {
                    cmd: &args.command,
                    brief: args.brief,
                    legacy_only: args.legacy_only,
                    heading_level: args.heading_level,
                    encoding: forced.map(|f| f.encoding.name()),
                    eol,
                    wsl: launcher.as_deref(),
                })
            );
            Ok(0)
        }
        Command::Info(args) => cmd_info(cli, &args.file, forced),
        Command::View(args) => cmd_view(cli, args, forced),
        Command::Search(args) => cmd_search(cli, args, forced),
        Command::Convert(args) => cmd_convert(cli, args, forced),
        Command::Create(args) => cmd_create(cli, args, forced),
        Command::Batch(args) => cmd_batch(cli, args, forced),
        _ => cmd_edit(cli, forced),
    }
}

// ---------------------------------------------------------------- read-only

fn cmd_encodings(cli: &Cli) {
    if cli.json {
        println!(
            "{}",
            json!({ "ok": true, "encodings": encoding_util::KNOWN_LABELS })
        );
        return;
    }
    println!("Supported encoding labels (WHATWG names and their usual aliases):");
    for label in encoding_util::KNOWN_LABELS {
        println!("  {label}");
    }
    println!(
        "\nAliases such as latin1, latin-1, iso-8859-1, cp1252 and ansi_x3.4-1968 are accepted.\n\
         Note: per the WHATWG standard, latin1/iso-8859-1 resolve to windows-1252, which is a\n\
         superset of ISO 8859-1 over the bytes 0x80-0x9F."
    );
}

fn cmd_guide(cli: &Cli, args: &cli::GuideArgs) -> Result<i32> {
    let text = match (&args.topic, args.list) {
        (_, true) => manual::topic_list(),
        (None, false) => manual::render_all(),
        (Some(topic), false) => match manual::find(topic) {
            Some(section) => manual::render_one(section),
            None => {
                return Err(AppError::new(
                    ErrorKind::Usage,
                    format!("unknown manual topic '{topic}'"),
                )
                .with_hint(format!(
                    "known topics: {}",
                    manual::SECTIONS
                        .iter()
                        .map(|s| s.key)
                        .collect::<Vec<_>>()
                        .join(", ")
                )))
            }
        },
    };

    if cli.json {
        println!(
            "{}",
            json!({
                "ok": true,
                "command": "guide",
                "topics": manual::SECTIONS.iter().map(|s| json!({
                    "topic": s.key,
                    "summary": s.summary,
                })).collect::<Vec<_>>(),
                "content": text,
            })
        );
    } else {
        print!("{text}");
    }
    Ok(0)
}

fn cmd_info(cli: &Cli, path: &Path, forced: Option<ForcedEncoding>) -> Result<i32> {
    let doc = Document::load(path, forced)?;
    let (eol, lf, crlf, cr) = lines::detect_eol(&doc.text);
    let resolved = document::link_target(&doc.path).map(|p| p.display().to_string());
    let mut value = json!({
        "ok": true,
        "command": "info",
        "path": doc.path.display().to_string(),
        "bytes": doc.raw.len(),
        "encoding": doc.encoding.name(),
        "detected_by": doc.detection.as_str(),
        "bom": doc.bom.is_some(),
        "eol": eol.name(),
        "eol_counts": { "lf": lf, "crlf": crlf, "cr": cr },
        "lines": doc.lines().count(),
        "characters": doc.text.chars().count(),
        "ends_with_newline": lines::ends_with_eol(&doc.text),
        "decode_errors": doc.had_decode_errors,
        "roundtrip_safe": doc.roundtrip,
        "looks_binary": doc.looks_binary(),
    });
    if let (Some(resolved), Some(obj)) = (&resolved, value.as_object_mut()) {
        obj.insert("resolved_path".into(), json!(resolved));
    }

    if cli.json {
        println!("{value}");
    } else {
        println!("path:            {}", doc.path.display());
        if let Some(resolved) = &resolved {
            println!("symlink to:      {resolved}");
        }
        println!("bytes:           {}", doc.raw.len());
        println!(
            "encoding:        {} (detected by: {})",
            doc.encoding.name(),
            doc.detection.as_str()
        );
        println!(
            "bom:             {}",
            if doc.bom.is_some() { "yes" } else { "no" }
        );
        println!(
            "line endings:    {} (lf={lf}, crlf={crlf}, cr={cr})",
            eol.name()
        );
        println!("lines:           {}", doc.lines().count());
        println!("characters:      {}", doc.text.chars().count());
        println!(
            "final newline:   {}",
            if lines::ends_with_eol(&doc.text) {
                "yes"
            } else {
                "no"
            }
        );
        println!(
            "edit safety:     {}",
            if doc.roundtrip {
                "byte-exact (edits keep every untouched byte)"
            } else if doc.had_decode_errors {
                "UNSAFE - the file has bytes that are invalid in this encoding"
            } else {
                "UNSAFE - the file does not round-trip through this encoding"
            }
        );
        if doc.looks_binary() {
            println!("warning:         file contains NUL bytes and may not be text");
        }
    }
    Ok(0)
}

fn cmd_view(cli: &Cli, args: &cli::ViewArgs, forced: Option<ForcedEncoding>) -> Result<i32> {
    let doc = Document::load(&args.file, forced)?;
    let index = doc.lines();

    let (first, last) = match &args.lines {
        Some(range) => {
            if index.count() == 0 {
                (1, 0)
            } else {
                range.resolve(index.count())?
            }
        }
        None => (1, index.count()),
    };

    let mut out = String::new();
    for n in first..=last {
        let line = match index.get(n) {
            Some(l) => l,
            None => break,
        };
        if args.number {
            out.push_str(&format!("{n:>6}\t"));
        }
        out.push_str(&doc.text[line.start..line.content_end]);
        out.push('\n');
    }

    if cli.json {
        println!(
            "{}",
            json!({
                "ok": true,
                "command": "view",
                "path": doc.path.display().to_string(),
                "encoding": doc.encoding.name(),
                "from_line": first,
                "to_line": last,
                "lines": index.count(),
                "content": out,
            })
        );
    } else {
        print!("{out}");
    }
    Ok(0)
}

fn cmd_search(cli: &Cli, args: &cli::SearchArgs, forced: Option<ForcedEncoding>) -> Result<i32> {
    let doc = Document::load(&args.file, forced)?;
    let pattern =
        textsrc::resolve_triple(&args.find, &args.find_file, false, "--find", cli.escapes)?;
    let ctx = ctx_for(cli, &doc)?;
    let needle = if args.regex {
        pattern.clone()
    } else {
        ctx.shape(&pattern)
    };

    let (value, count) = ops::search(&doc, args, &needle)?;

    if cli.json {
        let mut obj = value.as_object().cloned().unwrap_or_default();
        obj.insert("ok".into(), json!(true));
        obj.insert("command".into(), json!("search"));
        obj.insert("path".into(), json!(doc.path.display().to_string()));
        obj.insert("encoding".into(), json!(doc.encoding.name()));
        println!("{}", Value::Object(obj));
    } else if let Some(matches) = value.get("matches").and_then(|m| m.as_array()) {
        for m in matches {
            println!(
                "{}:{}:{}:{}",
                doc.path.display(),
                m["line"].as_u64().unwrap_or(0),
                m["column"].as_u64().unwrap_or(0),
                m["text"].as_str().unwrap_or("")
            );
        }
    }

    if count == 0 && !args.allow_empty {
        return Ok(ErrorKind::NoMatch.exit_code());
    }
    Ok(0)
}

// ------------------------------------------------------------------- edits

fn ctx_for(cli: &Cli, doc: &Document) -> Result<Ctx> {
    Ok(Ctx {
        eol_mode: resolve_eol_mode(cli)?,
        file_eol: doc.eol,
    })
}

/// Guard against silently mangling something that is not a text file.
fn preflight(cli: &Cli, doc: &Document) -> Result<()> {
    if doc.looks_binary() && !cli.force {
        return Err(AppError::new(
            ErrorKind::Encoding,
            format!(
                "{} contains NUL bytes and does not look like a text file",
                doc.path.display()
            ),
        )
        .with_hint("pass --force to edit it anyway"));
    }

    check_eol_mandate(cli, doc)?;

    // Under a project-wide encoding mandate, a guess is not good enough: a
    // wrong single-byte guess writes wrong bytes rather than failing.
    if doc.detection == Detection::Guessed && (cli.no_guess || env_flag(ENV_NO_GUESS)) {
        return Err(AppError::new(
            ErrorKind::Encoding,
            format!(
                "refusing to write: the encoding of {} was guessed ({}), not declared",
                doc.path.display(),
                doc.encoding.name()
            ),
        )
        .with_hint(format!(
            "pass --encoding LABEL, or set {ENV_ENCODING}=LABEL for every invocation"
        )));
    }
    Ok(())
}

/// Under `--strict-eol`, refuse to extend a file whose existing terminators are
/// not the mandated ones. Without this, appending CRLF text to an LF file
/// quietly produces a mixed-ending file.
fn check_eol_mandate(cli: &Cli, doc: &Document) -> Result<()> {
    if !(cli.strict_eol || env_flag(ENV_STRICT_EOL)) {
        return Ok(());
    }

    let mode = resolve_eol_mode(cli)?;
    let want = match mode {
        EolMode::Lf => Eol::Lf,
        EolMode::Crlf => Eol::CrLf,
        EolMode::Cr => Eol::Cr,
        // "auto" and "keep" follow the file, so there is nothing to enforce.
        EolMode::Auto | EolMode::Keep => {
            return Err(AppError::new(
                ErrorKind::Usage,
                format!(
                    "--strict-eol needs a line-ending style to enforce, but --eol is '{}'",
                    if mode == EolMode::Auto {
                        "auto"
                    } else {
                        "keep"
                    }
                ),
            )
            .with_hint(format!(
                "pass --eol lf|crlf|cr, or set {ENV_EOL} to one of them"
            )))
        }
    };

    // Commands that replace the whole file produce compliant output whatever
    // the old contents were, so judging them on the old contents is wrong —
    // and `convert --newlines` is the remedy this guard points people at.
    if matches!(
        cli.command,
        Command::Write(_) | Command::Create(_) | Command::Convert(_)
    ) {
        return Ok(());
    }

    let (_, lf, crlf, cr) = lines::detect_eol(&doc.text);
    let offending = match want {
        Eol::Lf => crlf + cr,
        Eol::CrLf => lf + cr,
        Eol::Cr => lf + crlf,
    };
    if offending == 0 {
        return Ok(());
    }

    Err(AppError::new(
        ErrorKind::Encoding,
        format!(
            "refusing to write: {} has {offending} line ending(s) that are not {} (lf={lf}, crlf={crlf}, cr={cr})",
            doc.path.display(),
            want.name().to_uppercase()
        ),
    )
    .with_hint(format!(
        "normalise it first: `intact convert {} --newlines {}`",
        doc.path.display(),
        want.name()
    )))
}

fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name)
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn cmd_edit(cli: &Cli, forced: Option<ForcedEncoding>) -> Result<i32> {
    let (path, command_name, allow_missing) = match &cli.command {
        Command::Replace(a) => (&a.file, "replace", false),
        Command::Insert(a) => (&a.file, "insert", false),
        Command::Append(a) => (&a.file, "append", false),
        Command::Prepend(a) => (&a.file, "prepend", false),
        Command::Delete(a) => (&a.file, "delete", false),
        Command::ReplaceLines(a) => (&a.file, "replace-lines", false),
        Command::Write(a) => (&a.file, "write", true),
        _ => unreachable!("cmd_edit called with a non-editing command"),
    };

    let doc = if allow_missing {
        Document::load_or_empty(path, forced)?
    } else {
        Document::load(path, forced)?
    };
    preflight(cli, &doc)?;
    let ctx = ctx_for(cli, &doc)?;

    let outcome = match &cli.command {
        Command::Replace(a) => {
            let find =
                textsrc::resolve_triple(&a.find, &a.find_file, false, "--find", cli.escapes)?;
            let with = if a.delete {
                String::new()
            } else {
                textsrc::resolve_triple(
                    &a.with,
                    &a.with_file,
                    a.with_stdin,
                    "--with (or --delete to remove the match)",
                    cli.escapes,
                )?
            };
            ops::replace(&doc, a, &find, &with, ctx)?
        }
        Command::Insert(a) => {
            let text = textsrc::resolve(&a.text, "insert", cli.escapes)?;
            ops::insert(&doc, a, &text, ctx)?
        }
        Command::Append(a) => {
            let text = textsrc::resolve(&a.text, "append", cli.escapes)?;
            ops::append(&doc, &text, ctx, !a.no_trailing_newline)?
        }
        Command::Prepend(a) => {
            let text = textsrc::resolve(&a.text, "prepend", cli.escapes)?;
            ops::prepend(&doc, &text, ctx, !a.no_trailing_newline)?
        }
        Command::Delete(a) => ops::delete(&doc, a)?,
        Command::ReplaceLines(a) => {
            let text = textsrc::resolve(&a.text, "replace-lines", cli.escapes)?;
            ops::replace_lines(&doc, a, &text, ctx)?
        }
        Command::Write(a) => {
            let text = textsrc::resolve(&a.text, "write", cli.escapes)?;
            ensure_parent_dir(&a.file, a.parents)?;
            ops::write_all(&doc, &text, ctx, !a.no_trailing_newline)?
        }
        _ => unreachable!(),
    };

    finish(cli, &doc, command_name, outcome)
}

fn cmd_create(cli: &Cli, args: &cli::CreateArgs, forced: Option<ForcedEncoding>) -> Result<i32> {
    let doc = Document::load_or_empty(&args.file, forced)?;
    if doc.existed && !args.overwrite {
        return Err(AppError::new(
            ErrorKind::Exists,
            format!("{} already exists", args.file.display()),
        )
        .with_hint("pass --overwrite, or use `intact write` to replace its contents"));
    }
    let ctx = ctx_for(cli, &doc)?;
    let text = textsrc::resolve(&args.text, "create", cli.escapes)?;
    ensure_parent_dir(&args.file, args.parents)?;
    let outcome = ops::write_all(&doc, &text, ctx, !args.no_trailing_newline)?;
    finish(cli, &doc, "create", outcome)
}

/// A new file's directory may not exist yet. Create it on request, and
/// otherwise say plainly which directory is missing.
fn ensure_parent_dir(file: &Path, create: bool) -> Result<()> {
    let dir = match file.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => return Ok(()),
    };
    if dir.is_dir() {
        return Ok(());
    }
    if create {
        std::fs::create_dir_all(dir).map_err(|e| {
            AppError::new(
                ErrorKind::Io,
                format!("cannot create {}: {e}", dir.display()),
            )
        })?;
        return Ok(());
    }
    Err(AppError::new(
        ErrorKind::NotFound,
        format!("directory {} does not exist", dir.display()),
    )
    .with_hint("pass --parents to create it"))
}

fn finish(cli: &Cli, doc: &Document, command: &'static str, outcome: OpOutcome) -> Result<i32> {
    let OpOutcome {
        mut edits,
        details,
        summary,
    } = outcome;
    let new_text = doc.apply_to_text(&edits);
    let bytes = doc.build_output(&mut edits, cli.unmappable, cli.lossy)?;
    // A brand-new file must be created even when its content is empty, so
    // "nothing to write" is not the same question as "bytes are unchanged".
    let changed = bytes != doc.raw || !doc.existed;

    let mut report = Report::new(command, doc);
    report.summary = summary;
    report.details = details;
    report.changed = changed;
    report.dry_run = cli.dry_run;
    report.bytes_after = bytes.len();
    report.lines_after = LineIndex::build(&new_text).count();
    // Report the line endings the file ends up with, not the ones it had: for a
    // new file those differ whenever --eol was used.
    report.eol = lines::detect_eol(&new_text).0.name();

    let hunk = if cli.dry_run {
        diff::diff(&doc.text, &new_text)
    } else {
        None
    };
    if let Some(h) = &hunk {
        let rendered = diff::render(h, 40);
        if cli.json {
            if let Value::Object(map) = &mut report.details {
                map.insert("diff".into(), json!(rendered));
            } else {
                report.details = json!({ "diff": rendered });
            }
        } else if !cli.quiet {
            print!("{rendered}");
        }
    }

    if !cli.dry_run && changed {
        doc.save(&bytes, cli.backup)?;
    }

    report::print_report(&report, cli.json, cli.quiet);
    Ok(0)
}

// ----------------------------------------------------------------- convert

fn cmd_convert(cli: &Cli, args: &cli::ConvertArgs, forced: Option<ForcedEncoding>) -> Result<i32> {
    let doc = Document::load(&args.file, forced)?;
    preflight(cli, &doc)?;
    // Omitting --to keeps the current encoding, which is what you want when
    // only --newlines is being changed.
    let target = match &args.to {
        Some(label) => encoding_util::encoding_for_label(label)?,
        None => doc.encoding,
    };

    if doc.had_decode_errors && !cli.lossy {
        return Err(AppError::new(
            ErrorKind::Encoding,
            format!(
                "{} has bytes that are not valid {}; converting would lose them",
                doc.path.display(),
                doc.encoding.name()
            ),
        )
        .with_hint(
            "pass --encoding LABEL to name the real source encoding, or --lossy to proceed",
        ));
    }

    let text = match args.newlines.unwrap_or(EolMode::Keep) {
        EolMode::Keep => doc.text.clone(),
        mode => lines::normalize_eol(&doc.text, mode.resolve(doc.eol).unwrap_or(Eol::Lf)),
    };

    let bom = match args.bom {
        BomMode::Add => BomKind::for_encoding(target),
        BomMode::Remove => None,
        BomMode::Keep => {
            if doc.bom.is_some() {
                BomKind::for_encoding(target)
            } else if target == encoding_rs::UTF_16LE || target == encoding_rs::UTF_16BE {
                // UTF-16 without a BOM is undetectable; always mark it.
                BomKind::for_encoding(target)
            } else {
                None
            }
        }
    };

    let mut bytes = Vec::new();
    if let Some(b) = bom {
        bytes.extend_from_slice(b.bytes());
    }
    bytes.extend_from_slice(&encode_text(target, &text, cli.unmappable)?);
    let changed = bytes != doc.raw;

    let new_eol = lines::detect_eol(&text).0;
    let mut parts = Vec::new();
    if target != doc.encoding {
        parts.push(format!("{} -> {}", doc.encoding.name(), target.name()));
    }
    if text != doc.text {
        parts.push(format!("line endings -> {}", new_eol.name()));
    }
    if parts.is_empty() {
        parts.push(format!("already {}", target.name()));
    }

    let mut report = Report::new("convert", &doc);
    report.summary = format!("converted: {}", parts.join(", "));
    report.eol = new_eol.name();
    report.changed = changed;
    report.dry_run = cli.dry_run;
    report.bytes_after = bytes.len();
    report.details = json!({
        "from_encoding": doc.encoding.name(),
        "to_encoding": target.name(),
        "from_eol": doc.eol.name(),
        "to_eol": new_eol.name(),
        "bom_before": doc.bom.is_some(),
        "bom_after": bom.is_some(),
    });

    if !cli.dry_run && changed {
        doc.save(&bytes, cli.backup)?;
    }
    report::print_report(&report, cli.json, cli.quiet);
    Ok(0)
}

// ------------------------------------------------------------------- batch

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum Script {
    Wrapped { ops: Vec<BatchOp> },
    Bare(Vec<BatchOp>),
}

#[derive(serde::Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
enum BatchOp {
    Replace {
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
        line: Option<Value>,
        #[serde(default)]
        after: Option<Value>,
        text: String,
    },
    Append {
        text: String,
    },
    Prepend {
        text: String,
    },
    Delete {
        lines: Value,
    },
    #[serde(rename = "replace-lines")]
    ReplaceLines {
        lines: Value,
        text: String,
    },
    Write {
        text: String,
    },
}

fn as_range(value: &Value) -> Result<LineRange> {
    let s = match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        other => {
            return Err(AppError::new(
                ErrorKind::Usage,
                format!("line range must be a string or number, got {other}"),
            ))
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
            ))
        }
    };
    s.parse::<lines::LineSpec>()
        .map_err(|e| AppError::new(ErrorKind::Usage, e))
}

fn cmd_batch(cli: &Cli, args: &cli::BatchArgs, forced: Option<ForcedEncoding>) -> Result<i32> {
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

    let original = Document::load(&args.file, forced)?;
    preflight(cli, &original)?;
    // Pin the encoding so that re-decoding between steps cannot drift, keeping
    // the original source so `detected_by` still reports the truth.
    let pinned = Some(ForcedEncoding {
        encoding: original.encoding,
        source: original.detection,
    });

    let mut current = original.raw.clone();
    let mut summaries = Vec::new();

    for (i, op) in batch_ops.iter().enumerate() {
        let doc = Document::from_bytes(args.file.clone(), current, pinned, true);
        let ctx = ctx_for(cli, &doc)?;
        let outcome = apply_batch_op(&doc, op, ctx).map_err(|e| AppError {
            message: format!("operation {}: {}", i + 1, e.message),
            ..e
        })?;
        let mut edits = outcome.edits;
        summaries.push(outcome.summary);
        current = doc.build_output(&mut edits, cli.unmappable, cli.lossy)?;
    }

    let changed = current != original.raw;
    let final_doc = Document::from_bytes(args.file.clone(), current.clone(), pinned, true);

    let mut report = Report::new("batch", &original);
    report.summary = format!("applied {} operation(s)", batch_ops.len());
    report.changed = changed;
    report.dry_run = cli.dry_run;
    report.bytes_after = current.len();
    report.lines_after = final_doc.lines().count();
    report.details = json!({ "operations": batch_ops.len(), "steps": summaries });

    if cli.dry_run {
        if let Some(h) = diff::diff(&original.text, &final_doc.text) {
            let rendered = diff::render(&h, 40);
            if cli.json {
                if let Value::Object(map) = &mut report.details {
                    map.insert("diff".into(), json!(rendered));
                }
            } else if !cli.quiet {
                print!("{rendered}");
            }
        }
    } else if changed {
        original.save(&current, cli.backup)?;
    }

    report::print_report(&report, cli.json, cli.quiet);
    Ok(0)
}

fn apply_batch_op(doc: &Document, op: &BatchOp, ctx: Ctx) -> Result<OpOutcome> {
    match op {
        BatchOp::Replace {
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
                with_stdin: false,
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
        BatchOp::Insert { line, after, text } => {
            let args = cli::InsertArgs {
                file: doc.path.clone(),
                line: line.as_ref().map(as_spec).transpose()?,
                after: after.as_ref().map(as_spec).transpose()?,
                text: cli::TextSource {
                    text: None,
                    text_file: None,
                    text_stdin: false,
                },
            };
            ops::insert(doc, &args, text, ctx)
        }
        BatchOp::Append { text } => ops::append(doc, text, ctx, true),
        BatchOp::Prepend { text } => ops::prepend(doc, text, ctx, true),
        BatchOp::Delete { lines: range } => {
            let args = cli::DeleteArgs {
                file: doc.path.clone(),
                lines: as_range(range)?,
            };
            ops::delete(doc, &args)
        }
        BatchOp::ReplaceLines { lines: range, text } => {
            let args = cli::ReplaceLinesArgs {
                file: doc.path.clone(),
                lines: as_range(range)?,
                text: cli::TextSource {
                    text: None,
                    text_file: None,
                    text_stdin: false,
                },
            };
            ops::replace_lines(doc, &args, text, ctx)
        }
        BatchOp::Write { text } => ops::write_all(doc, text, ctx, true),
    }
}
