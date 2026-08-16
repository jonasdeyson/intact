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

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use cli::{BomMode, Cli, Command};
use document::{BinaryPolicy, Detection, Document, ForcedEncoding};
use encoding_util::{BomKind, encode_text};
use error::{AppError, ErrorKind, Result};
use lines::{Eol, EolMode, LineIndex, LineRange};
use ops::{Ctx, OpOutcome};
use report::Report;

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
fn resolve_forced_encoding(cli: &Cli) -> Result<Option<ForcedEncoding>> {
    match &cli.encoding {
        Some(label) => Ok(Some(ForcedEncoding::flag(
            encoding_util::encoding_for_label(label)?,
        ))),
        None => Ok(None),
    }
}

/// `--eol` if given; otherwise match the file.
fn eol_mode(cli: &Cli) -> EolMode {
    cli.eol.unwrap_or(EolMode::Auto)
}

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
        Command::Batch(args) => cmd_batch(cli, args, forced),
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
    let resolved = document::link_target(&doc.path).map(|p| p.display().to_string());

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

fn ctx_for(cli: &Cli, doc: &Document) -> Ctx {
    Ctx {
        eol_mode: eol_mode(cli),
        file_eol: doc.eol,
    }
}

/// Whether this invocation will accept a file that does not look like text.
/// The guard applies to reads as well as writes, so every command resolves it
/// the same way; `info` is the one exception and passes `Allow` outright,
/// being the command that explains why the others refused.
fn binary_policy(cli: &Cli) -> BinaryPolicy {
    if cli.force {
        BinaryPolicy::Allow
    } else {
        BinaryPolicy::Refuse
    }
}

/// Guard against silently mangling something that is not a text file. The
/// binary check itself lives in `Document::load`, so that a file about to be
/// refused is never decoded.
fn preflight(cli: &Cli, doc: &Document) -> Result<()> {
    check_eol_mandate(cli, doc)?;

    // Under a project-wide encoding mandate, a guess is not good enough: a
    // wrong single-byte guess writes wrong bytes rather than failing.
    if doc.detection == Detection::Guessed && cli.no_guess {
        return Err(AppError::new(
            ErrorKind::Encoding,
            format!(
                "refusing to write: the encoding of {} was guessed ({}), not declared",
                doc.path.display(),
                doc.encoding.name()
            ),
        )
        .with_hint("pass --encoding LABEL to declare it"));
    }
    Ok(())
}

/// Under `--strict-eol`, refuse to extend a file whose existing terminators are
/// not the mandated ones. Without this, appending CRLF text to an LF file
/// quietly produces a mixed-ending file.
fn check_eol_mandate(cli: &Cli, doc: &Document) -> Result<()> {
    if !cli.strict_eol {
        return Ok(());
    }

    let mode = eol_mode(cli);
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
            .with_hint("pass --eol lf|crlf|cr"));
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

/// A diff is produced for `--dry-run` (where it is the whole point) and for
/// `--show-diff`, which reports an edit that was actually applied. The latter
/// is what puts the change in front of a human without a second command and a
/// second approval.
fn wants_diff(cli: &Cli) -> bool {
    cli.dry_run || cli.show_diff
}

fn merge_details(report: &mut Report, extra: Map<String, Value>) {
    match &mut report.details {
        Value::Object(map) => map.extend(extra),
        other => *other = Value::Object(extra),
    }
}

/// Print the diff above the summary line, or attach it to the JSON result.
/// Human output is capped; JSON is not, since it is not being read by eye.
fn emit_diff(cli: &Cli, report: &mut Report, diff: &diff::Diff, existed: bool, changed: bool) {
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

fn finish(cli: &Cli, doc: &Document, command: &'static str, outcome: OpOutcome) -> Result<i32> {
    let OpOutcome {
        mut edits,
        details,
        summary,
    } = outcome;
    // build_output sorts the edits, which apply_to_text and the diff both rely
    // on to walk the text in one pass.
    let bytes = doc.build_output(&mut edits, cli.unmappable, cli.lossy)?;
    let new_text = doc.apply_to_text(&edits);
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

    report::print_report(&report, cli.json, cli.quiet);
    Ok(0)
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

// ------------------------------------------------------------------- batch

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
        BatchOp::Write { file: _, text } => ops::write_all(doc, text, ctx, true),
    }
}
