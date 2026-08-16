use std::path::PathBuf;

use clap::{ArgGroup, Args, CommandFactory, FromArgMatches, Parser, Subcommand};

use crate::encoding_util::UnmappablePolicy;
use crate::lines::{EolMode, LineRange, LineSpec};

const AFTER_HELP: &str = "\
FULL DOCUMENTATION (this binary is self-documenting):
  intact guide              the complete manual: safety model, encodings,
                              line ranges, text input, JSON output, batch
                              scripts and worked recipes
  intact guide --list       the manual's topics
  intact guide TOPIC        one topic, e.g. `intact guide recipes`
  intact COMMAND --help     per-command options and examples
  intact instructions       a drop-in section for a project's CLAUDE.md

EXIT CODES:
  0  success
  1  generic failure
  2  bad arguments
  3  no match / target not found
  4  ambiguous match (more occurrences than allowed)
  5  encoding problem (undecodable file, or text unrepresentable in it)
  6  line number or range out of bounds
  7  file already exists (create)
  8  file not found
  9  I/O error

TEXT INPUT:
  Every command that takes text accepts --text/-t or --text-file PATH
  (`--text-file -` reads standard input). Text supplied to intact is always
  UTF-8; it is transcoded into the file's own encoding on write. --escapes
  interprets \\n, \\t, \\uXXXX in --text, --find and --with - and in text read
  from a file - so multi-line content fits in one argument.

LINE RANGES (--lines, --line, --after), 1-based and inclusive:
  7  5:9  5:  :9  $  3:$  -1  -3:-1

EXAMPLES:
  intact info notes.txt
  intact view src/main.c --lines 40:80 --number
  intact search src/app.py --find 'return None'
  intact replace config.ini --find 'debug=0' --with 'debug=1'
  intact replace src/app.py --find old_name --with new_name --all
  intact insert README.md --line 3 --text 'a new line'
  intact delete legacy.txt --lines 10:20
  intact replace-lines main.rs --lines 5:7 --text-file /tmp/block.txt
  intact convert legacy.txt --to utf-8
  intact --show-diff replace app.py --find x --with y   # global flags first
  intact batch --script ops.json                        # many edits, many files
";

#[derive(Parser, Debug)]
#[command(
    name = "intact",
    version,
    about = "Encoding-preserving text editor for files of any encoding, driven entirely by command-line arguments",
    long_about = "intact edits text files without changing their encoding.\n\n\
                  The file's encoding is detected (BOM, valid UTF-8, then statistical detection) \
                  or forced with --encoding. Text you pass on the command line is UTF-8 and is \
                  transcoded into the file's encoding, so editing a Latin-1 file leaves it \
                  Latin-1 and never double-encodes it. Bytes outside the edited region are \
                  written back byte-for-byte identical.\n\n\
                  Start with `intact guide` for the complete manual, or \
                  `intact COMMAND --help` for one command.",
    after_help = AFTER_HELP,
    // Keeps the global flags in their own block instead of interleaving them
    // with each subcommand's own options.
    next_help_heading = "GLOBAL OPTIONS"
)]
pub struct Cli {
    /// Emit a machine-readable JSON result on stdout
    #[arg(long, global = true)]
    pub json: bool,

    /// Print a unified diff of what would change, and write nothing
    #[arg(long, short = 'n', global = true)]
    pub dry_run: bool,

    /// Print a unified diff of the change as well as applying it
    #[arg(long, global = true)]
    pub show_diff: bool,

    /// Unchanged lines to show either side of a change, in a --dry-run or --show-diff diff
    #[arg(long, global = true, value_name = "N", default_value_t = crate::diff::DEFAULT_CONTEXT)]
    pub diff_context: usize,

    /// Copy the original file to FILE.bak before writing
    #[arg(long, global = true)]
    pub backup: bool,

    /// Force the file's encoding instead of detecting it (e.g. windows-1252, latin1, shift_jis)
    #[arg(long, short = 'e', global = true, value_name = "LABEL")]
    pub encoding: Option<String>,

    /// What to do with characters the file's encoding cannot represent
    #[arg(long, global = true, value_name = "POLICY", default_value = "error")]
    pub unmappable: UnmappablePolicy,

    /// Line endings to use for inserted text [default: auto]
    #[arg(long, global = true, value_name = "MODE")]
    pub eol: Option<EolMode>,

    /// Refuse to write to a file whose existing line endings differ from --eol
    #[arg(long, global = true)]
    pub strict_eol: bool,

    /// Interpret backslash escapes (\n, \t, \r, \0, \\, \xNN, \uXXXX) in supplied text, whether it
    /// came from --text/--find/--with or from their --text-file/--find-file/--with-file forms
    #[arg(long, global = true)]
    pub escapes: bool,

    /// Refuse to write to a file whose encoding was only statistically guessed
    #[arg(long, global = true)]
    pub no_guess: bool,

    /// Allow rewriting the whole file when it does not round-trip through its encoding
    #[arg(long, global = true)]
    pub lossy: bool,

    /// Read or edit even when the file does not look like text
    #[arg(long, global = true)]
    pub force: bool,

    /// Suppress the human-readable summary line
    #[arg(long, short = 'q', global = true)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Command,
}

// ------------------------------------------------- per-command global help
//
// The global options are declared once, on `Cli`, so that each one parses in
// either position: `intact --show-diff replace FILE ...` and
// `intact replace FILE ... --show-diff` mean the same thing. What clap charges
// for that is listing all fourteen of them under every subcommand, so
// `intact info --help` advertises --backup, --dry-run and --unmappable, none of
// which `info` reads.
//
// The table below says which globals each command actually honours, and
// `hide_unused_globals` hides the rest from that command's help. Only the help
// text changes: every global still parses everywhere, so a caller that puts
// `--encoding LABEL --no-guess` in front of every command uniformly - which
// `intact instructions --encoding LABEL` tells it to do - keeps working.

/// Every global option, by field name. `globals_table_is_complete` keeps this
/// in step with the struct above.
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

fn hide_unused_globals(mut cmd: clap::Command) -> clap::Command {
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

/// `Cli::parse()` with the per-command help filtering applied.
pub fn parse() -> Cli {
    let matches = hide_unused_globals(Cli::command()).get_matches();
    match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(err) => err.exit(),
    }
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Report a file's encoding, BOM, line endings and edit safety
    #[command(
        long_about = "Report what intact thinks a file is: its encoding and how that was \
                      determined, whether it has a BOM, its line endings, line and character \
                      counts, and whether byte-exact editing is available.\n\n\
                      Worth running first on anything unfamiliar. `detected_by: guessed` means \
                      statistical detection, which can be wrong on short files - see \
                      `intact guide encoding`.",
        after_help = "EXAMPLES:\n  \
                      intact info notes.txt\n  \
                      intact --json info notes.txt\n"
    )]
    Info(FileArgs),

    /// Print a file (or part of it) as UTF-8
    #[command(
        long_about = "Print a file, or a range of its lines, decoded to UTF-8 on stdout.\n\n\
                      The file itself is not modified and its own encoding is irrelevant to the \
                      output, which is always UTF-8.",
        after_help = "EXAMPLES:\n  \
                      intact view notes.txt\n  \
                      intact view src/main.c --lines 40:80 --number\n  \
                      intact view src/main.c --lines -20:-1\n"
    )]
    View(ViewArgs),

    /// Find occurrences of a string or regex, with line and column numbers
    #[command(
        long_about = "Find occurrences of a string or regex.\n\n\
                      Human output is `path:line:column:line-text`. Exits 3 when nothing \
                      matched, so it doubles as a test for whether an edit is applicable; pass \
                      --allow-empty for exit 0 instead.",
        after_help = "EXAMPLES:\n  \
                      intact search app.py --find 'return None'\n  \
                      intact search app.py --regex --find '^import ' --max 20\n  \
                      intact --json search app.py --find TODO --ignore-case\n"
    )]
    Search(SearchArgs),

    /// Replace occurrences of a string or regex
    #[command(
        long_about = "Replace occurrences of a string or regex.\n\n\
                      The match must be UNIQUE by default: if --find occurs more than once the \
                      command exits 4 and writes nothing, and if it occurs nowhere it exits 3. \
                      Widen that with --all, --occurrence N or --expect N, or narrow the search \
                      with --lines RANGE. This is the safe default for automated editing - it \
                      turns an ambiguous instruction into an error rather than a guess.",
        after_help = "EXAMPLES:\n  \
                      intact replace app.py --find 'timeout = 30' --with 'timeout = 60'\n  \
                      intact replace app.py --find old_name --with new_name --all\n  \
                      intact replace app.py --find x --with y --occurrence 2\n  \
                      intact replace app.py --find x --with y --expect 3\n  \
                      intact replace app.py --find x --with y --lines 40:80\n  \
                      intact replace app.py --find 'dead_code()' --delete\n  \
                      intact replace app.py --regex --all --find 'def (\\w+)\\(' --with 'def test_$1('\n\
                      \nSee `intact guide recipes` for more.\n"
    )]
    Replace(ReplaceArgs),

    /// Insert text before or after a line
    #[command(
        long_about = "Insert text as whole lines, before --line N or after --after N.\n\n\
                      A line terminator is added if the text lacks one, matching the file's \
                      dominant line ending. --line accepts one past the last line, meaning \
                      \"start a new line at the end of the file\".",
        after_help = "EXAMPLES:\n  \
                      intact insert app.py --line 1 --text 'import os'\n  \
                      intact insert app.py --after 40 --text-file /tmp/block.py\n  \
                      intact insert app.py --line 1 --escapes --text 'import os\\nimport sys'\n"
    )]
    Insert(InsertArgs),

    /// Add text at the end of the file
    #[command(
        long_about = "Append text at the end of the file.\n\n\
                      If the file does not already end with a line terminator, one is added \
                      first, so the appended text always starts on its own line.",
        after_help = "EXAMPLES:\n  \
                      intact append CHANGELOG.md --text '- fixed the thing'\n  \
                      cat block.txt | intact append notes.txt --text-file -\n"
    )]
    Append(TextOnlyArgs),

    /// Add text at the start of the file
    #[command(after_help = "EXAMPLES:\n  \
                            intact prepend main.rs --text '// SPDX-License-Identifier: MIT'\n")]
    Prepend(TextOnlyArgs),

    /// Delete a range of lines
    #[command(
        long_about = "Delete a range of lines, including their terminators.\n\n\
                      Ranges are 1-based and inclusive: see `intact guide ranges`.",
        after_help = "EXAMPLES:\n  \
                      intact delete app.py --lines 120:145\n  \
                      intact delete app.py --lines 7\n  \
                      intact delete app.py --lines -3:-1     # the last three lines\n"
    )]
    Delete(DeleteArgs),

    /// Replace a range of lines with new text
    #[command(
        name = "replace-lines",
        // Hidden rather than visible: a second name for one command is one more
        // thing to disambiguate, and it earns nothing in the help output.
        alias = "set-lines",
        long_about = "Replace a range of lines with new text.\n\n\
                      If the replaced region ended with a line terminator the replacement gets \
                      one too, so replacing the last line of a file that lacks a final newline \
                      does not add one.",
        after_help = "EXAMPLES:\n  \
                      intact replace-lines app.py --lines 20:35 --text-file /tmp/new_block.py\n  \
                      intact replace-lines app.py --lines 5 --text 'x = 1'\n  \
                      intact replace-lines app.py --lines 3:$ --text 'tail'\n"
    )]
    ReplaceLines(ReplaceLinesArgs),

    /// Replace the entire contents of a file, keeping its encoding
    #[command(
        long_about = "Replace the entire contents of a file, keeping its encoding.\n\n\
                      The file is created if it does not exist (as UTF-8, or as --encoding). \
                      Use `create` instead when overwriting an existing file would be a bug.",
        after_help = "EXAMPLES:\n  \
                      intact write notes.txt --text-file /tmp/new.txt\n  \
                      intact write notes.txt --text-file - < /tmp/new-2.txt\n"
    )]
    Write(WriteArgs),

    /// Create a new file, failing if it already exists
    #[command(after_help = "EXAMPLES:\n  \
                            intact create src/new.rs --text 'fn main() {}'\n  \
                            intact --encoding windows-1252 create legacy.txt --text 'café'\n")]
    Create(CreateArgs),

    /// Re-encode a file into a different encoding
    #[command(
        long_about = "Re-encode a file into a different encoding.\n\n\
                      This is the one command that intentionally rewrites every byte. Use it to \
                      migrate a legacy file to UTF-8 before inserting characters its old \
                      encoding cannot represent. UTF-16 output always gets a BOM.",
        after_help = "EXAMPLES:\n  \
                      intact convert legacy.txt --to utf-8\n  \
                      intact convert legacy.txt --to utf-8 --bom remove\n  \
                      intact --encoding windows-1252 convert legacy.txt --to utf-8\n  \
                      intact convert notes.txt --to utf-8 --newlines lf\n"
    )]
    Convert(ConvertArgs),

    /// Apply several operations to one or more files from a JSON script
    #[command(
        long_about = "Apply several operations from a JSON script, with one write per file.\n\n\
                      This is the way to make more than one edit in a single command. \
                      Operations run in order and each sees the result of the previous one, so \
                      line numbers refer to the state at that step. If any operation fails, \
                      nothing is written at all — not for that file, and not for any other.\n\n\
                      An operation may carry its own \"file\", which is how one script edits \
                      several files; FILE on the command line is the default for the operations \
                      that do not. This is the only multi-file mode: every other command takes \
                      one file, so that each file's encoding is decided and reported separately. \
                      See `intact guide batch` for the schema.",
        after_help = "EXAMPLE SCRIPT:\n  \
                      {\"ops\": [\n    \
                        {\"op\": \"replace\", \"find\": \"DEBUG = True\", \"with\": \"DEBUG = False\"},\n    \
                        {\"op\": \"delete\", \"lines\": \"40:42\"},\n    \
                        {\"op\": \"insert\", \"line\": 1, \"text\": \"# generated\"},\n    \
                        {\"op\": \"replace\", \"file\": \"other.py\", \"find\": \"x\", \"with\": \"y\"}\n  \
                      ]}\n\nEXAMPLES:\n  \
                      intact batch app.py --script ops.json\n  \
                      intact batch app.py --script -          # script on stdin\n  \
                      intact batch --script ops.json          # every op names its own file\n"
    )]
    Batch(BatchArgs),

    /// Print the complete manual, or one topic of it
    #[command(
        long_about = "Print the built-in manual.\n\n\
                      With no topic, the whole manual is printed. `--list` names the topics; \
                      passing a topic prints just that section. `intact guide encoding` ends \
                      with the list of encoding labels this build understands.",
        after_help = "EXAMPLES:\n  \
                      intact guide\n  \
                      intact guide --list\n  \
                      intact guide recipes\n  \
                      intact guide encoding      # detection, labels, converting\n"
    )]
    Guide(GuideArgs),

    /// Print a ready-to-paste CLAUDE.md / AGENTS.md section describing this tool
    #[command(
        alias = "claude-md",
        long_about = "Print a Markdown section documenting intact for another project's agent \
                      instructions file (CLAUDE.md, AGENTS.md, .cursorrules, ...).\n\n\
                      Append the output to the target project's instructions file so that an \
                      agent working there knows the tool exists, when to reach for it, and how \
                      to read its exit codes.\n\n\
                      If the target project mandates one encoding for every file, pass \
                      --encoding LABEL: the generated section then tells the agent to pass \
                      --encoding and --no-guess on every command instead of letting detection \
                      guess.\n\n\
                      If the agent works from Windows while this binary lives in WSL, pass \
                      --wsl. The generated section then opens with the rule that every command \
                      in it is prefixed with `wsl.exe`, and covers the traps that come with \
                      that: WSL paths, PATH, and Windows shell quoting.",
        after_help = "EXAMPLES:\n  \
                      intact instructions >> CLAUDE.md\n  \
                      intact instructions --brief >> AGENTS.md\n  \
                      intact instructions --command /opt/bin/intact >> CLAUDE.md\n  \
                      intact instructions --encoding latin1 >> CLAUDE.md\n  \
                      intact instructions --legacy-only >> CLAUDE.md\n  \
                      intact instructions --wsl >> CLAUDE.md\n  \
                      intact instructions --wsl Ubuntu-24.04 >> CLAUDE.md\n"
    )]
    Instructions(InstructionsArgs),
}

#[derive(Args, Debug)]
pub struct GuideArgs {
    /// Topic to print; omit for the whole manual
    #[arg(value_name = "TOPIC")]
    pub topic: Option<String>,

    /// List the available topics
    #[arg(long, short = 'l', conflicts_with = "topic")]
    pub list: bool,
}

#[derive(Args, Debug)]
pub struct InstructionsArgs {
    /// Emit a short version (a handful of lines instead of a full section)
    #[arg(long, short = 'b')]
    pub brief: bool,

    /// How the binary should be invoked in the generated text
    #[arg(long, short = 'c', value_name = "NAME", default_value = "intact")]
    pub command: String,

    /// Narrow the mandate to non-UTF-8 files only (the default covers every file edit)
    #[arg(long)]
    pub legacy_only: bool,

    /// Heading level for the generated section (1-4)
    #[arg(long, value_name = "N", default_value_t = 2, value_parser = clap::value_parser!(u8).range(1..=4))]
    pub heading_level: u8,

    /// The agent runs on Windows and this binary lives in WSL: prefix every
    /// command with `wsl.exe` (optionally `-d DISTRO`) and explain WSL paths
    #[arg(long, value_name = "DISTRO", num_args = 0..=1)]
    pub wsl: Option<Option<String>>,
}

#[derive(Args, Debug)]
pub struct FileArgs {
    /// File to inspect
    pub file: PathBuf,
}

/// Counts of occurrences and matches. Zero is never a sensible answer for any
/// of them - `--max 0` finds nothing and reports "no match", `--expect 0` can
/// only ever exit 3 or 4, `--occurrence 0` names no occurrence - so they are
/// rejected as the arguments rather than as results.
fn at_least_one(value: &str) -> Result<usize, String> {
    match value.parse::<usize>() {
        Ok(n) if n > 0 => Ok(n),
        Ok(_) => Err("must be 1 or more".to_string()),
        Err(_) => Err(format!("`{value}` is not a whole number")),
    }
}

/// `--text` / `--text-file`. Every command that flattens this needs the text,
/// so the pair is a required group: the usage line then says so, rather than
/// each command discovering it once it has already opened the file.
#[derive(Args, Debug, Clone)]
#[group(required = true, multiple = false)]
pub struct TextSource {
    /// Text to use (UTF-8)
    #[arg(long, short = 't', value_name = "TEXT", allow_hyphen_values = true)]
    pub text: Option<String>,

    /// Read the text from a UTF-8 file ("-" for standard input)
    #[arg(long, value_name = "PATH")]
    pub text_file: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct TextOnlyArgs {
    /// File to edit
    pub file: PathBuf,

    #[command(flatten)]
    pub text: TextSource,

    /// Do not ensure the result ends with a line terminator
    #[arg(long)]
    pub no_trailing_newline: bool,
}

#[derive(Args, Debug)]
pub struct WriteArgs {
    /// File to write
    pub file: PathBuf,

    #[command(flatten)]
    pub text: TextSource,

    /// Create missing parent directories
    #[arg(long, short = 'p')]
    pub parents: bool,

    /// Do not ensure the result ends with a line terminator
    #[arg(long)]
    pub no_trailing_newline: bool,
}

#[derive(Args, Debug)]
pub struct CreateArgs {
    /// File to create
    pub file: PathBuf,

    #[command(flatten)]
    pub text: TextSource,

    /// Create missing parent directories
    #[arg(long, short = 'p')]
    pub parents: bool,

    /// Do not ensure the result ends with a line terminator
    #[arg(long)]
    pub no_trailing_newline: bool,
}

#[derive(Args, Debug)]
pub struct ViewArgs {
    /// File to print
    pub file: PathBuf,

    /// Line range to print, e.g. 5, 5:9, 5:, :9, 3:$, -5:-1
    #[arg(long, short = 'l', value_name = "RANGE", allow_hyphen_values = true)]
    pub lines: Option<LineRange>,

    /// Prefix each line with its number
    #[arg(long, short = 'N')]
    pub number: bool,
}

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// File to search
    pub file: PathBuf,

    /// Text to look for
    #[arg(long, short = 'f', value_name = "TEXT", allow_hyphen_values = true)]
    pub find: Option<String>,

    /// Read the search text from a UTF-8 file ("-" for standard input)
    #[arg(long, value_name = "PATH", conflicts_with = "find")]
    pub find_file: Option<PathBuf>,

    /// Treat the search text as a regular expression
    #[arg(long, short = 'r')]
    pub regex: bool,

    /// Case-insensitive matching
    #[arg(long, short = 'i')]
    pub ignore_case: bool,

    /// Restrict the search to a line range
    #[arg(long, short = 'l', value_name = "RANGE", allow_hyphen_values = true)]
    pub lines: Option<LineRange>,

    /// Stop after this many matches
    #[arg(long, short = 'm', value_name = "N", value_parser = at_least_one)]
    pub max: Option<usize>,

    /// Exit 0 with no output even when nothing matches
    #[arg(long)]
    pub allow_empty: bool,
}

#[derive(Args, Debug)]
pub struct ReplaceArgs {
    /// File to edit
    pub file: PathBuf,

    /// Text to replace
    #[arg(long, short = 'f', value_name = "TEXT", allow_hyphen_values = true)]
    pub find: Option<String>,

    /// Read the search text from a UTF-8 file ("-" for standard input)
    #[arg(long, value_name = "PATH", conflicts_with = "find")]
    pub find_file: Option<PathBuf>,

    /// Replacement text
    #[arg(
        long = "with",
        short = 'w',
        value_name = "TEXT",
        allow_hyphen_values = true
    )]
    pub with: Option<String>,

    /// Read the replacement from a UTF-8 file ("-" for standard input)
    #[arg(long = "with-file", value_name = "PATH", conflicts_with = "with")]
    pub with_file: Option<PathBuf>,

    /// Remove the matched text instead of replacing it
    #[arg(long, conflicts_with_all = ["with", "with_file"])]
    pub delete: bool,

    /// Treat the search text as a regular expression ($1, ${name} expand in the replacement)
    #[arg(long, short = 'r')]
    pub regex: bool,

    /// Case-insensitive matching
    #[arg(long, short = 'i')]
    pub ignore_case: bool,

    /// Replace every occurrence (without this, the match must be unique)
    #[arg(long, short = 'a')]
    pub all: bool,

    /// Replace only the Nth occurrence (1-based)
    #[arg(long, value_name = "N", conflicts_with = "all", value_parser = at_least_one)]
    pub occurrence: Option<usize>,

    /// Require exactly N occurrences, and replace them all
    #[arg(long, value_name = "N", conflicts_with_all = ["all", "occurrence"], value_parser = at_least_one)]
    pub expect: Option<usize>,

    /// Restrict the replacement to a line range
    #[arg(long, short = 'l', value_name = "RANGE", allow_hyphen_values = true)]
    pub lines: Option<LineRange>,

    /// Do not expand $1 / ${name} in a regex replacement
    #[arg(long, requires = "regex")]
    pub no_expand: bool,
}

/// An insert has to say where, so `--line` and `--after` are one required
/// group rather than two options that happen to be checked once the file is
/// already open. (`batch` builds these args from JSON, which clap never sees,
/// so `ops::insert` still checks for itself.)
#[derive(Args, Debug)]
#[command(group = ArgGroup::new("at").required(true).args(["line", "after"]))]
pub struct InsertArgs {
    /// File to edit
    pub file: PathBuf,

    /// Insert before this line (may be one past the last line to append)
    #[arg(long, short = 'l', value_name = "LINE", allow_hyphen_values = true)]
    pub line: Option<LineSpec>,

    /// Insert after this line
    #[arg(long, short = 'a', value_name = "LINE", allow_hyphen_values = true)]
    pub after: Option<LineSpec>,

    #[command(flatten)]
    pub text: TextSource,
}

#[derive(Args, Debug)]
pub struct DeleteArgs {
    /// File to edit
    pub file: PathBuf,

    /// Lines to delete, e.g. 5, 5:9, 5:, 3:$, -3:-1
    #[arg(long, short = 'l', value_name = "RANGE", allow_hyphen_values = true)]
    pub lines: LineRange,
}

#[derive(Args, Debug)]
pub struct ReplaceLinesArgs {
    /// File to edit
    pub file: PathBuf,

    /// Lines to replace, e.g. 5, 5:9, 3:$
    #[arg(long, short = 'l', value_name = "RANGE", allow_hyphen_values = true)]
    pub lines: LineRange,

    #[command(flatten)]
    pub text: TextSource,
}

#[derive(Args, Debug)]
pub struct ConvertArgs {
    /// File to convert
    pub file: PathBuf,

    /// Target encoding label; omit to keep the current encoding and only change --newlines
    #[arg(long, value_name = "LABEL", required_unless_present = "newlines")]
    pub to: Option<String>,

    /// Byte-order mark handling in the output
    #[arg(long, value_name = "MODE", default_value = "keep")]
    pub bom: BomMode,

    /// Also rewrite line endings
    #[arg(long, value_name = "MODE")]
    pub newlines: Option<EolMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum BomMode {
    /// Keep a BOM if the file had one and the target supports it
    Keep,
    /// Write a BOM
    Add,
    /// Write no BOM
    Remove,
}

#[derive(Args, Debug)]
pub struct BatchArgs {
    /// Default file for operations that do not name a "file" of their own
    pub file: Option<PathBuf>,

    /// JSON script describing the operations ("-" for standard input)
    #[arg(long, short = 's', value_name = "PATH")]
    pub script: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;
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
