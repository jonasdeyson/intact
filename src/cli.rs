use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

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

ENVIRONMENT:
  INTACT_ENCODING   Encoding to use for every invocation, as if --encoding
                      were passed. --encoding still overrides it. Set this in a
                      project that mandates one encoding, e.g.
                      `export INTACT_ENCODING=latin1`, so that detection never
                      runs and new files are created in that encoding too.
  INTACT_NO_GUESS   Set to 1 to refuse writing to any file whose encoding was
                      only statistically guessed (same as --no-guess).
  INTACT_EOL        Line endings for every invocation, as if --eol were
                      passed: auto, lf, crlf, cr or keep. Set this in a project
                      that mandates one style, so new files get it too.
  INTACT_STRICT_EOL Set to 1 to refuse writing to any file whose existing line
                      endings differ from the mandated ones (same as
                      --strict-eol). Requires --eol lf|crlf|cr.
  INTACT_SHOW_DIFF  Set to 1 to print a unified diff of every edit as it is
                      applied (same as --show-diff), so that a change is
                      visible without a separate preview command.

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
  Every command that takes text accepts --text/-t, --text-file PATH, or
  --text-stdin. Text supplied to intact is always UTF-8; it is transcoded
  into the file's own encoding on write. --escapes interprets \\n, \\t, \\uXXXX
  so multi-line content fits in one argument.

LINE RANGES (--lines, --line, --after), 1-based and inclusive:
  7  5:9  5:  :9  $  3:$  -1  -3:-1  5..9

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

    /// Unchanged lines to show either side of a change in a diff
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

    /// Line endings to use for inserted text [default: auto, or $INTACT_EOL]
    #[arg(long, global = true, value_name = "MODE")]
    pub eol: Option<EolMode>,

    /// Refuse to write to a file whose existing line endings differ from --eol
    #[arg(long, global = true)]
    pub strict_eol: bool,

    /// Interpret backslash escapes (\n, \t, \r, \0, \\, \xNN, \uXXXX) in --text and --find
    #[arg(long, global = true)]
    pub escapes: bool,

    /// Refuse to write to a file whose encoding was only statistically guessed
    #[arg(long, global = true)]
    pub no_guess: bool,

    /// Allow rewriting the whole file when it does not round-trip through its encoding
    #[arg(long, global = true)]
    pub lossy: bool,

    /// Edit even when the file looks binary (contains NUL bytes)
    #[arg(long, global = true)]
    pub force: bool,

    /// Suppress the human-readable summary line
    #[arg(long, short = 'q', global = true)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Command,
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
                      cat block.txt | intact append notes.txt --text-stdin\n"
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
        visible_alias = "set-lines",
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
                      intact write notes.txt --text-stdin < /tmp/new.txt\n"
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

    /// Apply several operations atomically from a JSON script
    #[command(
        long_about = "Apply several operations with a single atomic write.\n\n\
                      Operations run in order and each sees the result of the previous one, so \
                      line numbers refer to the state at that step. If any operation fails, \
                      nothing is written at all. See `intact guide batch` for the schema.",
        after_help = "EXAMPLE SCRIPT:\n  \
                      {\"ops\": [\n    \
                        {\"op\": \"replace\", \"find\": \"DEBUG = True\", \"with\": \"DEBUG = False\"},\n    \
                        {\"op\": \"delete\", \"lines\": \"40:42\"},\n    \
                        {\"op\": \"insert\", \"line\": 1, \"text\": \"# generated\"}\n  \
                      ]}\n\nEXAMPLES:\n  \
                      intact batch app.py --script ops.json\n  \
                      intact batch app.py --script -\n"
    )]
    Batch(BatchArgs),

    /// List the encoding labels this build understands
    Encodings,

    /// Print the complete manual, or one topic of it
    #[command(
        long_about = "Print the built-in manual.\n\n\
                      With no topic, the whole manual is printed. `--list` names the topics; \
                      passing a topic prints just that section.",
        after_help = "EXAMPLES:\n  \
                      intact guide\n  \
                      intact guide --list\n  \
                      intact guide recipes\n  \
                      intact guide encoding\n"
    )]
    Guide(GuideArgs),

    /// Print a ready-to-paste CLAUDE.md / AGENTS.md section describing this tool
    #[command(
        visible_alias = "claude-md",
        long_about = "Print a Markdown section documenting intact for another project's agent \
                      instructions file (CLAUDE.md, AGENTS.md, .cursorrules, ...).\n\n\
                      Append the output to the target project's instructions file so that an \
                      agent working there knows the tool exists, when to reach for it, and how \
                      to read its exit codes.\n\n\
                      If the target project mandates one encoding for every file, pass \
                      --encoding LABEL: the generated section then tells the agent to pin that \
                      encoding (via INTACT_ENCODING and INTACT_NO_GUESS, or the flag on \
                      every command) instead of letting detection guess.\n\n\
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

/// `--text` / `--text-file` / `--text-stdin`.
#[derive(Args, Debug, Clone)]
pub struct TextSource {
    /// Text to use (UTF-8)
    #[arg(long, short = 't', value_name = "TEXT", allow_hyphen_values = true)]
    pub text: Option<String>,

    /// Read the text from a UTF-8 file
    #[arg(long, value_name = "PATH", conflicts_with = "text")]
    pub text_file: Option<PathBuf>,

    /// Read the text from standard input (UTF-8)
    #[arg(long, conflicts_with_all = ["text", "text_file"])]
    pub text_stdin: bool,
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

    /// Overwrite the file if it already exists
    #[arg(long)]
    pub overwrite: bool,

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

    /// Read the search text from a UTF-8 file
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
    #[arg(long, short = 'm', value_name = "N")]
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

    /// Read the search text from a UTF-8 file
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

    /// Read the replacement from a UTF-8 file
    #[arg(long = "with-file", value_name = "PATH", conflicts_with = "with")]
    pub with_file: Option<PathBuf>,

    /// Read the replacement from standard input
    #[arg(long = "with-stdin", conflicts_with_all = ["with", "with_file"])]
    pub with_stdin: bool,

    /// Remove the matched text instead of replacing it
    #[arg(long, conflicts_with_all = ["with", "with_file", "with_stdin"])]
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
    #[arg(long, value_name = "N", conflicts_with = "all")]
    pub occurrence: Option<usize>,

    /// Require exactly N occurrences, and replace them all
    #[arg(long, value_name = "N", conflicts_with_all = ["all", "occurrence"])]
    pub expect: Option<usize>,

    /// Restrict the replacement to a line range
    #[arg(long, short = 'l', value_name = "RANGE", allow_hyphen_values = true)]
    pub lines: Option<LineRange>,

    /// Do not expand $1 / ${name} in a regex replacement
    #[arg(long)]
    pub no_expand: bool,
}

#[derive(Args, Debug)]
pub struct InsertArgs {
    /// File to edit
    pub file: PathBuf,

    /// Insert before this line (may be one past the last line to append)
    #[arg(long, short = 'l', value_name = "LINE", allow_hyphen_values = true)]
    pub line: Option<LineSpec>,

    /// Insert after this line
    #[arg(
        long,
        short = 'a',
        value_name = "LINE",
        allow_hyphen_values = true,
        conflicts_with = "line"
    )]
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
    /// File to edit
    pub file: PathBuf,

    /// JSON script describing the operations ("-" for standard input)
    #[arg(long, short = 's', value_name = "PATH")]
    pub script: PathBuf,
}
