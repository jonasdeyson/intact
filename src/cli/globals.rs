//! Which global options each subcommand advertises in its own `--help`.
//!
//! The global options are declared once, on `Cli`, so that each one parses in
//! either position: `intact --show-diff replace FILE ...` and
//! `intact replace FILE ... --show-diff` mean the same thing. What clap charges
//! for that is listing all fourteen of them under every subcommand, so
//! `intact info --help` advertises --backup, --dry-run and --unmappable, none of
//! which `info` reads.
//!
//! The table below says which globals each command actually honours, and
//! `hide_unused_globals` hides the rest from that command's help. Only the help
//! text changes: every global still parses everywhere, so a caller that puts
//! `--encoding LABEL --no-guess` in front of every command uniformly - which
//! `intact instructions --encoding LABEL` tells it to do - keeps working.

/// Every global option, by field name. `globals_table_is_complete` keeps this
/// in step with the `Cli` struct.
const ALL_GLOBALS: &[&str] = &[
    "json",
    "dry_run",
    "show_diff",
    "diff_context",
    "backup",
    "encoding",
    "unmappable",
    "eol",
    "strict_eol",
    "escapes",
    "no_guess",
    "lossy",
    "force",
    "quiet",
];

/// Opening a file at all, and reporting what happened.
const FILE: &[&str] = &["json", "encoding"];
/// Writing one: preview it, and say less about it.
const WRITE: &[&str] = &["dry_run", "show_diff", "diff_context", "quiet"];
/// Keeping the previous contents of a file that already existed.
const BACKUP: &[&str] = &["backup"];
/// The guards that refuse to write to an existing file, and their overrides.
const GUARD: &[&str] = &["no_guess", "force", "lossy"];
/// The binary guard alone, which refuses reads too — so the read-only commands
/// advertise its override without the write-only guards beside it.
const BINARY: &[&str] = &["force"];
/// Encoding text intact adds into the file's own encoding.
const ENCODE: &[&str] = &["unmappable"];
/// Line endings for text intact adds.
const EOL: &[&str] = &["eol"];
/// Enforcing one line-ending style, which needs `--eol` to name it.
const MANDATE: &[&str] = &["eol", "strict_eol"];
/// Backslash escapes in `--text`, `--find` and `--with`.
const ESCAPES: &[&str] = &["escapes"];

/// The groups each subcommand honours. Everything else is hidden from its help.
#[rustfmt::skip]
const GLOBALS_BY_COMMAND: &[(&str, &[&[&str]])] = &[
    // The binary guard never stops `info`; --force instead means what it means
    // everywhere else - treat the file as text - and prints the text-level
    // report that a non-text file otherwise withholds.
    ("info",          &[FILE, BINARY]),
    ("view",          &[FILE, BINARY]),
    ("search",        &[FILE, BINARY, ESCAPES]),
    ("replace",       &[FILE, WRITE, BACKUP, GUARD, ENCODE, MANDATE, ESCAPES]),
    ("insert",        &[FILE, WRITE, BACKUP, GUARD, ENCODE, MANDATE, ESCAPES]),
    ("append",        &[FILE, WRITE, BACKUP, GUARD, ENCODE, MANDATE, ESCAPES]),
    ("prepend",       &[FILE, WRITE, BACKUP, GUARD, ENCODE, MANDATE, ESCAPES]),
    ("replace-lines", &[FILE, WRITE, BACKUP, GUARD, ENCODE, MANDATE, ESCAPES]),
    // Replaces the whole content, so --strict-eol has nothing to enforce: the
    // output is compliant whatever the file held before. Same as `create`.
    ("write",         &[FILE, WRITE, BACKUP, GUARD, ENCODE, EOL, ESCAPES]),
    // Deletes nothing but whole lines: no new text is encoded, and there is no
    // --text to unescape. --strict-eol still guards the write.
    ("delete",        &[FILE, WRITE, BACKUP, GUARD, MANDATE]),
    // The file cannot already exist, so there is nothing to back up, nothing
    // whose encoding was guessed, and no existing line endings to enforce.
    ("create",        &[FILE, WRITE, ENCODE, EOL, ESCAPES]),
    // Line endings are `convert --newlines`, not the global --eol.
    ("convert",       &[FILE, WRITE, BACKUP, GUARD, ENCODE]),
    // Text comes from JSON, which has escapes of its own.
    ("batch",         &[FILE, WRITE, BACKUP, GUARD, ENCODE, MANDATE]),
    ("guide",         &[&["json"]]),
    // --encoding and --eol are read as "this project mandates X" and end up in
    // the generated text.
    ("instructions",  &[&["encoding", "eol"]]),
];

pub fn hide_unused_globals(mut cmd: clap::Command) -> clap::Command {
    // Globals live on the parent until `build` copies them into each
    // subcommand, and it is those copies that a subcommand's help renders - so
    // the hiding has to happen after the build. `mut_args` is the only way to
    // reach them at that point: it rewrites the arguments in place, where
    // anything that removes and re-adds one (`mut_arg`) leaves clap's
    // long-flag lookup table pointing at the wrong arguments.
    cmd.build();
    for sub in cmd.get_subcommands_mut() {
        let name = sub.get_name().to_owned();
        // Anything absent from the table - clap's own `help` subcommand - is
        // left as it is.
        let Some((_, groups)) = GLOBALS_BY_COMMAND.iter().find(|(n, _)| *n == name) else {
            continue;
        };
        let shown: Vec<&str> = groups.iter().flat_map(|g| g.iter().copied()).collect();
        let built = std::mem::take(sub);
        *sub = built.mut_args(|arg| {
            let id = arg.get_id().as_str();
            if ALL_GLOBALS.contains(&id) && !shown.contains(&id) {
                arg.hide(true)
            } else {
                arg
            }
        });
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use clap::{CommandFactory, FromArgMatches};
    use std::collections::HashSet;

    fn global_ids() -> HashSet<String> {
        Cli::command()
            .get_arguments()
            .filter(|a| a.is_global_set())
            .map(|a| a.get_id().to_string())
            .collect()
    }

    /// A global that nothing lists is invisible everywhere; an id that is not a
    /// global (a typo, or a renamed field) makes `mut_arg` panic at startup.
    #[test]
    fn globals_table_is_complete() {
        let actual = global_ids();
        let listed: HashSet<String> = ALL_GLOBALS.iter().map(|s| s.to_string()).collect();
        assert_eq!(actual, listed, "ALL_GLOBALS is out of step with Cli");

        for (name, groups) in GLOBALS_BY_COMMAND {
            for id in groups.iter().flat_map(|g| g.iter()) {
                assert!(
                    listed.contains(*id),
                    "{name}: '{id}' is not a global option"
                );
            }
        }
    }

    /// Every subcommand needs an entry, or it keeps the unfiltered list.
    #[test]
    fn every_subcommand_is_in_the_table() {
        for sub in Cli::command().get_subcommands() {
            let name = sub.get_name();
            assert!(
                GLOBALS_BY_COMMAND.iter().any(|(n, _)| *n == name),
                "{name} is missing from GLOBALS_BY_COMMAND"
            );
        }
    }

    #[test]
    fn irrelevant_globals_are_hidden_but_still_parse() {
        let cmd = hide_unused_globals(Cli::command());
        let info = cmd
            .get_subcommands()
            .find(|s| s.get_name() == "info")
            .expect("info subcommand");
        let hidden: HashSet<&str> = info
            .get_arguments()
            .filter(|a| a.is_hide_set())
            .map(|a| a.get_id().as_str())
            .collect();
        assert!(hidden.contains("backup"));
        assert!(hidden.contains("dry_run"));
        assert!(!hidden.contains("json"));
        assert!(!hidden.contains("encoding"));

        // Hiding is a help-only change: the flag is still accepted, in either
        // position, so existing scripts keep working.
        let cmd = hide_unused_globals(Cli::command());
        let matches = cmd
            .try_get_matches_from(["intact", "info", "--backup", "f.txt"])
            .expect("--backup still parses on info");
        assert!(Cli::from_arg_matches(&matches).unwrap().backup);
    }
}
