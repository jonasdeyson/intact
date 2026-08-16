mod atomic;
mod batch;
mod binary;
mod cli;
mod diff;
mod document;
mod encoding_util;
mod error;
mod lines;
mod manual;
mod ops;
mod policy;
mod report;
mod textsrc;

use std::path::Path;

use serde_json::{Value, json};

use atomic::ensure_parent_dir;
use binary::BinaryPolicy;
use cli::{BomMode, Cli, Command};
use document::{Document, ForcedEncoding};
use encoding_util::{BomKind, encode_text};
use error::{AppError, ErrorKind, Result};
use lines::{Eol, EolMode};
use policy::{binary_policy, ctx_for, eol_mode, preflight, resolve_forced_encoding};
use report::{Report, emit_diff, finish, wants_diff};

fn main() {
    let cli = cli::parse();
    let json_mode = cli.json;
    match run(&cli) {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            report::print_error(&err, json_mode);
            std::process::exit(err.kind.exit_code());
        }
    }
}

/// Every setting comes from the command line. There is deliberately no
/// environment fallback: an agent typically runs each command in a fresh shell,
/// so an `export` in one invocation is gone by the next, and a setting that
/// silently applies only sometimes is worse than no setting at all.
fn run(cli: &Cli) -> Result<i32> {
    let forced = resolve_forced_encoding(cli)?;

    match &cli.command {
        Command::Guide(args) => cmd_guide(cli, args),
        Command::Instructions(args) => {
            // The global --encoding and --eol double as "this project mandates
            // X"; they arrive here already validated and canonicalised.
            let eol = match eol_mode(cli) {
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
        Command::Batch(args) => batch::run(cli, args, forced),
        _ => cmd_edit(cli, forced),
    }
}

// ---------------------------------------------------------------- read-only

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
                )));
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
    // `info` never refuses: it is how a caller finds out why everything else did.
    let doc = Document::load(path, forced, BinaryPolicy::Allow)?;
    let resolved = atomic::link_target(&doc.path).map(|p| p.display().to_string());

    // Everything below `bytes` describes the file decoded as text: an encoding
    // detection run over it, the line endings of the result, how many
    // characters it came to. For a file that is not text, not one of those is
    // a fact about the file - the encoding is a guess about a blob, and the
    // line endings are however many 0x0A bytes happened to fall in it. So the
    // whole text-level report is withheld rather than qualified: a reader who
    // has to be told which of twelve numbers to disregard has been handed the
    // work `info` exists to do. `--force` means "treat this as text" here as
    // everywhere else, and prints it in full.
    let text_report = doc.binary.is_none() || cli.force;

    let mut value = json!({
        "ok": true,
        "command": "info",
        "path": doc.path.display().to_string(),
        "bytes": doc.raw.len(),
        "looks_binary": doc.looks_binary(),
    });
    if let (Some(resolved), Some(obj)) = (&resolved, value.as_object_mut()) {
        obj.insert("resolved_path".into(), json!(resolved));
    }
    if let (Some(hint), Some(obj)) = (doc.binary, value.as_object_mut()) {
        obj.insert(
            "binary".into(),
            json!({
                "reason": hint.reason(),
                "offset": hint.offset(),
                "detail": hint.describe(),
            }),
        );
    }

    let (eol, lf, crlf, cr) = lines::detect_eol(&doc.text);
    let mojibake = doc.mojibake();
    if let Some(obj) = value.as_object_mut().filter(|_| text_report) {
        obj.insert("encoding".into(), json!(doc.encoding.name()));
        obj.insert("detected_by".into(), json!(doc.detection.as_str()));
        obj.insert("bom".into(), json!(doc.bom.is_some()));
        obj.insert("eol".into(), json!(eol.name()));
        obj.insert(
            "eol_counts".into(),
            json!({ "lf": lf, "crlf": crlf, "cr": cr }),
        );
        obj.insert("lines".into(), json!(doc.lines().count()));
        obj.insert("characters".into(), json!(doc.text.chars().count()));
        obj.insert(
            "ends_with_newline".into(),
            json!(lines::ends_with_eol(&doc.text)),
        );
        obj.insert("decode_errors".into(), json!(doc.had_decode_errors));
        obj.insert("roundtrip_safe".into(), json!(doc.roundtrip));
        if let Some(hint) = &mojibake {
            obj.insert(
                "mojibake".into(),
                json!({
                    "count": hint.count,
                    "line": doc.lines().line_of_offset(hint.offset),
                    "sample": hint.sample,
                }),
            );
        }
    }

    if cli.json {
        println!("{value}");
        return Ok(0);
    }

    println!("path:            {}", doc.path.display());
    if let Some(resolved) = &resolved {
        println!("symlink to:      {resolved}");
    }
    println!("bytes:           {}", doc.raw.len());
    if let Some(hint) = doc.binary {
        let tail = if text_report {
            "The report below reads it as text, because --force was given."
        } else {
            "The rest of this report would describe a decoding of the bytes \
             rather than the file, and is withheld."
        };
        println!(
            "not text:        {}",
            wrap_indented(
                &format!(
                    "{}. Every command but `info` refuses it - {}. {tail}",
                    hint.describe(),
                    hint.fix()
                ),
                78,
                17
            )
        );
    }
    if !text_report {
        return Ok(0);
    }

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
    if let Some(warning) = doc.mojibake_warning() {
        println!("warning:         {}", wrap_indented(&warning, 78, 17));
    }
    Ok(0)
}

/// Wrap `text` to `width` columns, indenting every line after the first by
/// `indent` spaces so it lines up under a label the caller already printed.
fn wrap_indented(text: &str, width: usize, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let mut out = String::new();
    let mut col = indent;
    for word in text.split_whitespace() {
        let len = word.chars().count();
        if col > indent && col + 1 + len > width {
            out.push('\n');
            out.push_str(&pad);
            col = indent;
        } else if col > indent {
            out.push(' ');
            col += 1;
        }
        out.push_str(word);
        col += len;
    }
    out
}

fn cmd_view(cli: &Cli, args: &cli::ViewArgs, forced: Option<ForcedEncoding>) -> Result<i32> {
    let doc = Document::load(&args.file, forced, binary_policy(cli))?;
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
    let doc = Document::load(&args.file, forced, binary_policy(cli))?;
    let pattern = textsrc::resolve_pair(
        &args.find,
        &args.find_file,
        "--find (or --find-file) is required",
        cli.escapes,
    )?;
    let ctx = ctx_for(cli, &doc);
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

fn cmd_edit(cli: &Cli, forced: Option<ForcedEncoding>) -> Result<i32> {
    let (path, command_name, allow_missing) = match &cli.command {
        Command::Replace(a) => (&a.file, "replace", false),
        Command::Insert(a) => (&a.file, "insert", false),
        Command::Append(a) => (&a.file, "append", false),
        Command::Prepend(a) => (&a.file, "prepend", false),
        Command::Delete(a) => (&a.file, "delete", false),
        Command::ReplaceLines(a) => (&a.file, "replace-lines", false),
        Command::MoveLines(a) => (&a.file, "move-lines", false),
        Command::Write(a) => (&a.file, "write", true),
        _ => unreachable!("cmd_edit called with a non-editing command"),
    };

    let doc = if allow_missing {
        Document::load_or_empty(path, forced, binary_policy(cli))?
    } else {
        Document::load(path, forced, binary_policy(cli))?
    };
    preflight(cli, &doc)?;
    let ctx = ctx_for(cli, &doc);

    let outcome = match &cli.command {
        Command::Replace(a) => {
            // There is only one standard input: the first read drains it and
            // the second silently gets an empty string, which for --with-file
            // means deleting the match rather than replacing it.
            let stdin = Some(Path::new("-"));
            if a.find_file.as_deref() == stdin && a.with_file.as_deref() == stdin {
                return Err(AppError::new(
                    ErrorKind::Usage,
                    "--find-file and --with-file cannot both read standard input",
                )
                .with_hint("pass one of them inline as --find/--with, or from a file"));
            }
            let find = textsrc::resolve_pair(
                &a.find,
                &a.find_file,
                "--find (or --find-file) is required",
                cli.escapes,
            )?;
            let with = if a.delete {
                String::new()
            } else {
                textsrc::resolve_pair(
                    &a.with,
                    &a.with_file,
                    "--with (or --with-file, or --delete to remove the match) is required",
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
        Command::MoveLines(a) => ops::move_lines(&doc, a, ctx)?,
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
    let doc = Document::load_or_empty(&args.file, forced, binary_policy(cli))?;
    // Refusing outright is the whole difference between `create` and `write`;
    // an --overwrite flag here would just be a second spelling of `write`.
    if doc.existed {
        return Err(AppError::new(
            ErrorKind::Exists,
            format!("{} already exists", args.file.display()),
        )
        .with_hint("use `intact write` to replace its contents"));
    }
    let ctx = ctx_for(cli, &doc);
    let text = textsrc::resolve(&args.text, "create", cli.escapes)?;
    ensure_parent_dir(&args.file, args.parents)?;
    let outcome = ops::write_all(&doc, &text, ctx, !args.no_trailing_newline)?;
    finish(cli, &doc, "create", outcome)
}

// ----------------------------------------------------------------- convert

fn cmd_convert(cli: &Cli, args: &cli::ConvertArgs, forced: Option<ForcedEncoding>) -> Result<i32> {
    let doc = Document::load(&args.file, forced, binary_policy(cli))?;
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

    // Only the Unicode encodings have a byte-order mark. Silently dropping an
    // explicit --bom add would leave the caller believing the file is marked.
    if args.bom == BomMode::Add && BomKind::for_encoding(target).is_none() {
        return Err(AppError::new(
            ErrorKind::Usage,
            format!("{} has no byte-order mark to add", target.name()),
        )
        .with_hint("only UTF-8 and UTF-16 have one"));
    }

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

    // A conversion has no edit spans to work from: it rewrites terminators
    // across the whole file, or only re-encodes it and changes no text at all.
    if wants_diff(cli) {
        let diff = diff::from_texts(&doc.text, &text, cli.diff_context);
        emit_diff(cli, &mut report, &diff, doc.existed, changed);
    }

    if !cli.dry_run && changed {
        doc.save(&bytes, cli.backup)?;
    }
    report::print_report(&report, cli.json, cli.quiet);
    Ok(0)
}
