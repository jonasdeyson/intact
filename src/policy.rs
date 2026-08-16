//! Turning command-line flags into the decisions every command shares:
//! which encoding to impose, which line endings to write, and whether a file
//! may be written to at all.

use crate::binary::BinaryPolicy;
use crate::cli::{Cli, Command};
use crate::document::{Detection, Document, ForcedEncoding};
use crate::encoding_util;
use crate::error::{AppError, ErrorKind, Result};
use crate::lines::{self, Eol, EolMode};
use crate::ops::Ctx;

pub fn resolve_forced_encoding(cli: &Cli) -> Result<Option<ForcedEncoding>> {
    match &cli.encoding {
        Some(label) => Ok(Some(ForcedEncoding::flag(
            encoding_util::encoding_for_label(label)?,
        ))),
        None => Ok(None),
    }
}

/// `--eol` if given; otherwise match the file.
pub fn eol_mode(cli: &Cli) -> EolMode {
    cli.eol.unwrap_or(EolMode::Auto)
}

pub fn ctx_for(cli: &Cli, doc: &Document) -> Ctx {
    Ctx {
        eol_mode: eol_mode(cli),
        file_eol: doc.eol,
    }
}

/// Whether this invocation will accept a file that does not look like text.
/// The guard applies to reads as well as writes, so every command resolves it
/// the same way; `info` is the one exception and passes `Allow` outright,
/// being the command that explains why the others refused.
pub fn binary_policy(cli: &Cli) -> BinaryPolicy {
    if cli.force {
        BinaryPolicy::Allow
    } else {
        BinaryPolicy::Refuse
    }
}

/// Guard against silently mangling something that is not a text file. The
/// binary check itself lives in `Document::load`, so that a file about to be
/// refused is never decoded.
pub fn preflight(cli: &Cli, doc: &Document) -> Result<()> {
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
