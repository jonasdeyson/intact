//! The built-in manual, served by `intact guide [TOPIC]`.
//!
//! The executable is meant to be self-documenting: an agent handed nothing but
//! the binary must be able to discover the entire API starting from `--help`.
//! Everything the README says lives here too, so the two never drift apart in
//! the only direction that matters — the binary is the source of truth.

pub struct Section {
    pub key: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
    pub body: &'static str,
}

pub const SECTIONS: &[Section] = &[
    Section {
        key: "overview",
        title: "OVERVIEW",
        summary: "what intact is for and how a session typically goes",
        body: "\
intact edits text files driven entirely by command-line arguments, without ever
changing a file's encoding.

It exists for AI coding agents. An agent works in UTF-8 internally, so writing a
file back naively re-encodes everything as UTF-8 and turns a Latin-1 `café` into
`cafÃ©`. intact takes UTF-8 text on the command line, transcodes it into
whatever encoding the target file already uses, and splices it in without
touching any other byte.

A typical session:

  intact info src/legacy.c              # what encoding is this, is it safe?
  intact view src/legacy.c --lines 40:80 --number
  intact search src/legacy.c --find 'malloc('
  intact replace src/legacy.c --find 'malloc(n)' --with 'calloc(n, 1)'

Every command accepts --help. `intact guide` prints this whole manual;
`intact guide TOPIC` prints one section; `intact guide --list` names them.

COMMANDS

  Inspect (never write):
    info           encoding, BOM, line endings, line count, edit safety
    view           print a file or a line range as UTF-8
    search         find a string or regex, with line and column numbers
    encodings      list supported encoding labels

  Edit:
    replace        replace occurrences of a string or regex
    insert         insert text before or after a line
    append         add text at the end of the file
    prepend        add text at the start of the file
    delete         delete a line range
    replace-lines  replace a line range with new text
    write          replace the entire contents, keeping the encoding
    create         create a new file, failing if it exists
    convert        re-encode the file into a different encoding
    batch          apply several operations with one atomic write

  Documentation:
    guide          this manual
    instructions   a section to paste into a project's CLAUDE.md / AGENTS.md
    help           per-command help (also `intact CMD --help`)

SCOPE

intact writes file contents: it creates, edits and truncates files, and
--parents will create a missing directory for a new file. It deliberately does
not delete, rename, move or copy files, and does not change permissions — use
the normal tools for those. It also works on one file per invocation; there is
no glob or multi-file mode, so that each file's encoding is decided and
reported separately.",
    },
    Section {
        key: "safety",
        title: "SAFETY MODEL",
        summary: "the guarantees, and what makes a command refuse to write",
        body: "\
GUARANTEES

1. Encoding is preserved. The file's encoding is detected (BOM, then valid
   UTF-8, then statistical detection) or forced with --encoding. Output is
   written in that same encoding.

2. Untouched bytes stay identical. When the file round-trips through its
   encoding, edits are applied by splicing encoded bytes into the original
   buffer. Everything outside the edited region is the byte that was already
   there, so repeated edits cannot accumulate mojibake.

3. No silent corruption. If the file does not round-trip, or if your text
   contains a character the file's encoding cannot represent, the command fails
   with a distinct exit code and writes nothing.

4. Line endings and BOMs are preserved. Inserted text is rewritten to the file's
   dominant line ending; a BOM stays exactly as it was; a missing final newline
   stays missing.

5. Writes are atomic. Content goes to a temporary file in the same directory,
   then is renamed over the original, carrying its permissions. Editing a
   symlink writes the file it points at and leaves the link intact; the result
   line names both (`link.txt -> real.txt: updated ...`), so an edit landing
   somewhere other than the path you typed is never silent.

WHEN A COMMAND REFUSES

  refusing to edit: FILE does not round-trip through ENCODING

    Detection picked an encoding that cannot reproduce the original bytes, or
    the file is damaged. Run `intact info FILE` and pass the right --encoding.
    --lossy rewrites the whole file anyway, accepting that untouched parts may
    change. Exit code 5.

  character 'X' (U+NNNN) cannot be represented in ENCODING

    Your replacement text needs a character the file's encoding lacks. Either
    `intact convert FILE --to utf-8` first, or pick a substitution policy with
    --unmappable replace|xml|skip. Exit code 5.

  N occurrences of text \"...\"; refusing to guess

    replace requires a unique match by default. Pass --all, --occurrence N,
    --lines RANGE, or extend --find until it is unique. Exit code 4.

  FILE contains NUL bytes and does not look like a text file

    Pass --force if you really mean it. Exit code 5.

DRY RUNS

--dry-run prints a diff of what would change and writes nothing. It is the
cheapest way to confirm an edit does what you intended before committing to it.",
    },
    Section {
        key: "encoding",
        title: "ENCODINGS",
        summary: "detection, labels, and forcing an encoding",
        body: "\
DETECTION ORDER

  1. --encoding LABEL, if given, wins outright.
  2. The INTACT_ENCODING environment variable.
  3. A byte-order mark (UTF-8, UTF-16LE, UTF-16BE).
  4. Bytes that are valid UTF-8 are treated as UTF-8.
  5. Otherwise chardetng guesses a legacy encoding.

`intact info FILE` reports which of these applied, as `detected_by`:
explicit, environment, bom, utf-8-valid, guessed, or default (empty/new file).

PROJECTS THAT MANDATE ONE ENCODING

If every file in a project must be, say, Latin-1, do not rely on detection at
all — and do not rely on remembering --encoding on each command either:

  export INTACT_ENCODING=latin1
  export INTACT_NO_GUESS=1

INTACT_ENCODING applies to every invocation, including `create`, which
otherwise makes UTF-8 files. --encoding still overrides it for the odd file that
is genuinely different.

INTACT_NO_GUESS (or --no-guess) makes any *write* to a file whose encoding was
merely guessed fail with exit 5 instead of proceeding. Read-only commands (info,
view, search) still work, so a file that trips the guard can still be diagnosed.

That pairing matters more than it looks. A wrong single-byte guess does not
merely display the file oddly: existing bytes survive, but text you insert is
encoded in the wrong repertoire. Inserting 'ć' into a file guessed as
windows-1250 writes byte 0xE6, which a Latin-1 reader shows as 'æ'. Pinning the
encoding removes that whole class of failure.

`intact instructions --encoding latin1` generates a CLAUDE.md section stating
the policy, for agents working in such a project.

GUESSES ARE GUESSES

Statistical detection needs a reasonable amount of text. On a short file with
one or two non-ASCII bytes it may land on the wrong single-byte encoding —
windows-1250 instead of windows-1252, say. Bytes already in the file are still
safe, because every single-byte encoding round-trips and nothing existing gets
rewritten. But the character repertoire differs, so inserting 'ã' into a file
believed to be windows-1250 fails with exit 5 and a message saying the encoding
was guessed rather than declared.

When a project's encoding is known, pass --encoding — or set INTACT_ENCODING
once and add INTACT_NO_GUESS=1 so a missed flag fails loudly instead of
falling back to a guess.

LABELS

Labels follow the WHATWG Encoding Standard: utf-8, utf-16le, utf-16be,
windows-1250 through windows-1258, windows-874, iso-8859-2 through iso-8859-16,
koi8-r, koi8-u, macintosh, x-mac-cyrillic, ibm866, gbk, gb18030, big5, euc-jp,
shift_jis, iso-2022-jp, euc-kr, and their usual aliases (latin1, cp1252, ...).
Run `intact encodings` for the list.

Two things worth knowing:

  * Per the standard, latin1 / iso-8859-1 resolve to windows-1252, which differs
    from strict ISO 8859-1 only in how bytes 0x80-0x9F are named. Both
    round-trip, so the bytes on disk are unaffected either way.

  * iso-2022-jp is stateful, so byte-exact splicing is unavailable for it. Such
    files need --lossy, which re-encodes the whole file.

CONVERTING

  intact convert FILE --to utf-8
  intact convert FILE --to utf-8 --bom remove
  intact convert FILE --to utf-16le            # a BOM is added automatically
  intact convert FILE --to utf-8 --newlines lf # also normalise line endings

--bom keep (default) preserves whether the file had one; add and remove force it.
UTF-16 output always gets a BOM, since UTF-16 without one is undetectable.",
    },
    Section {
        key: "ranges",
        title: "LINE NUMBERS AND RANGES",
        summary: "the syntax accepted by --lines, --line and --after",
        body: "\
Line numbers are 1-based. Ranges are inclusive at both ends.

  7        line 7
  5:9      lines 5 through 9
  5:       line 5 to the end of the file
  :9       the start of the file through line 9
  $        the last line (`end` and `last` also work)
  3:$      line 3 to the last line
  -1       the last line
  -3:-1    the last three lines
  5..9     same as 5:9

--line and --after take a single position (7, $, -2); --lines takes a range.

`insert --line N` accepts one past the last line, meaning \"start a new line at
the end of the file\". `insert --after N` requires an existing line.

A range that runs backwards, or names a line beyond the end of the file, exits 6.

Commands that accept --lines as a filter rather than a target — replace and
search — restrict the operation to that region and leave the rest alone.",
    },
    Section {
        key: "text",
        title: "SUPPLYING TEXT",
        summary: "--text, --text-file, --text-stdin, and backslash escapes",
        body: "\
Input text is always UTF-8. It is transcoded into the file's own encoding on
write, which is the entire point of this tool.

Every command that takes text accepts one of:

  --text TEXT, -t     the argument itself
  --text-file PATH    a UTF-8 file
  --text-stdin        standard input

replace uses a matching set for each half:

  --find TEXT, -f     --with TEXT, -w
  --find-file PATH    --with-file PATH
                      --with-stdin
                      --delete          remove the match instead of replacing it

ESCAPES

--escapes interprets backslash sequences in --text and --find. This is the
easiest way to pass multi-line content in a single argument:

  intact insert main.rs --line 1 --escapes --text 'use std::fmt;\\nuse std::io;'

Supported: \\n \\r \\t \\0 \\\\ \\' \\\" \\xNN \\uXXXX \\u{XXXXX}

Without --escapes, a literal backslash in your text is just a backslash, which
is what you want when editing code containing regex or Windows paths.

LINE ENDINGS IN INSERTED TEXT

--eol controls the terminators of text you supply:

  auto   (default) rewrite them to match the file's dominant line ending
  lf     crlf     cr     force one
  keep   insert the text exactly as given

Under auto, a literal (non-regex) --find is also rewritten, so searching for
'a\\nb' works on a CRLF file.

PROJECTS THAT MANDATE ONE LINE-ENDING STYLE

auto is right for a repository of mixed files and wrong for one with a policy:
it follows each file rather than the policy, and a brand-new file created with
auto always gets LF. As with encodings, pin it instead of remembering a flag:

  export INTACT_EOL=crlf
  export INTACT_STRICT_EOL=1

INTACT_EOL applies to every invocation, including `create`. --eol still
overrides it.

INTACT_STRICT_EOL (or --strict-eol) makes any write to a file whose existing
terminators are not the mandated ones fail with exit 5, instead of appending
CRLF text to an LF file and leaving it mixed. It reports the counts it found:

  refusing to write: app.c has 2 line ending(s) that are not CRLF
  (lf=2, crlf=0, cr=0)
  hint: normalise it first: `intact convert app.c --newlines crlf`

The guard needs something to enforce, so --eol must be lf, crlf or cr; with auto
or keep it is a usage error rather than a silent no-op. write, create and
convert are exempt, because they produce compliant output whatever the file
held before.

NORMALISING AN EXISTING FILE

  intact convert FILE --newlines crlf   # --to is optional here
  intact convert FILE --newlines auto   # collapse mixed endings to the
                                          # file's own dominant style

--to may be omitted when --newlines is given, so line endings can be fixed
without restating (or knowing) the file's encoding.

TRAILING NEWLINES

append, prepend, write and create ensure the result ends with a line terminator;
--no-trailing-newline turns that off. replace-lines keeps whatever the replaced
region had, so replacing the last line of a file that lacks a final newline does
not add one.",
    },
    Section {
        key: "exit-codes",
        title: "EXIT CODES",
        summary: "one code per failure class, for branching without parsing prose",
        body: "\
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

`search` exits 3 when nothing matched; pass --allow-empty for exit 0 instead.

Every failing command writes nothing. A refusal never leaves the file in a
half-edited state, and batch is all-or-nothing across all of its operations.",
    },
    Section {
        key: "json",
        title: "JSON OUTPUT",
        summary: "machine-readable results and errors",
        body: "\
--json makes every command emit one JSON object. Successful results go to
stdout, failures to stderr, and the process exit code is unchanged.

SUCCESS (edits)

  {\"ok\":true,\"command\":\"replace\",\"path\":\"a.txt\",\"encoding\":\"windows-1252\",
   \"detected_by\":\"guessed\",\"bom\":false,\"eol\":\"lf\",\"changed\":true,
   \"dry_run\":false,\"bytes_before\":30,\"bytes_after\":31,\"lines_before\":3,
   \"lines_after\":3,\"summary\":\"replaced 1 of 1 occurrence(s)\",
   \"occurrences_found\":1,\"occurrences_replaced\":1,\"first_line\":1,
   \"first_column\":4}

Command-specific fields are merged into the same object. --dry-run adds a
\"diff\" field instead of writing.

A \"resolved_path\" field appears (in edit and info results alike) only when
the path given is a symlink, and holds the file actually read and written.
Its absence means the path is the file.

SUCCESS (search)

  {\"ok\":true,\"command\":\"search\",\"count\":2,\"matches\":[
     {\"line\":1,\"column\":4,\"offset\":3,\"match\":\"é\",\"text\":\"café\"}]}

SUCCESS (info)

  {\"ok\":true,\"command\":\"info\",\"bytes\":30,\"encoding\":\"windows-1252\",
   \"detected_by\":\"guessed\",\"bom\":false,\"eol\":\"lf\",
   \"eol_counts\":{\"lf\":3,\"crlf\":0,\"cr\":0},\"lines\":3,\"characters\":30,
   \"ends_with_newline\":true,\"decode_errors\":false,\"roundtrip_safe\":true,
   \"looks_binary\":false}

FAILURE (on stderr)

  {\"ok\":false,\"error\":\"...\",\"kind\":\"ambiguous\",\"hint\":\"...\",\"exit_code\":4}

\"kind\" is one of: other, usage, no_match, ambiguous, encoding, range, exists,
not_found, io — the same taxonomy as the exit codes.",
    },
    Section {
        key: "batch",
        title: "BATCH SCRIPTS",
        summary: "several edits, one atomic write, all-or-nothing",
        body: "\
  intact batch FILE --script ops.json
  intact batch FILE --script -        # read the script from stdin

Operations are applied in order. Later operations see the results of earlier
ones, so line numbers refer to the state at that step. If any operation fails,
nothing is written at all.

  {
    \"ops\": [
      { \"op\": \"replace\", \"find\": \"DEBUG = True\", \"with\": \"DEBUG = False\" },
      { \"op\": \"replace\", \"find\": \"log(\", \"with\": \"logger.info(\", \"all\": true },
      { \"op\": \"delete\", \"lines\": \"40:42\" },
      { \"op\": \"insert\", \"line\": 1, \"text\": \"# generated\" },
      { \"op\": \"append\", \"text\": \"# end\" }
    ]
  }

A bare JSON array works too. Fields per op:

  replace        find, with, regex, ignore_case, all, occurrence, expect,
                 lines, no_expand
  insert         line or after, text
  append         text
  prepend        text
  delete         lines
  replace-lines  lines, text
  write          text

\"lines\" and \"line\" accept a number or any range string from the ranges topic.
Unknown fields are rejected, so a typo fails loudly instead of being ignored.

Text in a script is UTF-8 JSON, with normal JSON escapes — use \\n for newlines
rather than the --escapes flag.",
    },
    Section {
        key: "recipes",
        title: "RECIPES",
        summary: "worked examples for common editing tasks",
        body: "\
INSPECT BEFORE EDITING AN UNFAMILIAR FILE

  intact info notes.txt
  intact view notes.txt --lines 1:40 --number

CHANGE ONE UNIQUE LINE (the safe default)

  intact replace app.py --find 'timeout = 30' --with 'timeout = 60'

Exits 4 if that text appears more than once, 3 if it appears nowhere.

RENAME A SYMBOL EVERYWHERE

  intact replace app.py --find old_name --with new_name --all
  intact replace app.py --regex --all --find '\\bold_name\\b' --with new_name

DISAMBIGUATE A REPEATED STRING

  intact search app.py --find 'return None'          # see where they are
  intact replace app.py --find 'return None' --with 'return []' --lines 40:80
  intact replace app.py --find 'return None' --with 'return []' --occurrence 2

INSERT AN IMPORT AT THE TOP

  intact insert app.py --line 1 --text 'import os'

INSERT RELATIVE TO AN ANCHOR, WITHOUT KNOWING A LINE NUMBER

Rewrite the anchor as itself plus the new line, rather than looking the line
number up first:

  intact replace app.py --escapes \\
      --find 'import b' --with 'import b\\nimport c'

The same trick inserts before an anchor ('...\\nimport b'), and it keeps the
edit anchored to content rather than to a position that may have moved.

CREATE A FILE IN A DIRECTORY THAT DOES NOT EXIST YET

  intact create src/components/Foo.tsx --parents --text 'export const Foo = () => null;'

Without --parents a missing directory is an error (exit 8) rather than a
silently created tree.

THE SAME CHANGE ACROSS SEVERAL FILES

There is no glob or multi-file mode: one file per invocation, so that each
file's encoding is decided and reported separately. Loop in the shell:

  for f in src/*.py; do intact replace \"$f\" --find old --with new --all; done

Note that a file where the text does not appear exits 3, which will show up in
the loop; use `intact search` first if you want to skip those quietly.

REPLACE A BLOCK OF LINES WITH A FILE'S CONTENTS

  intact replace-lines app.py --lines 20:35 --text-file /tmp/new_block.py

INSERT MULTI-LINE TEXT IN ONE ARGUMENT

  intact insert app.py --line 1 --escapes --text 'import os\\nimport sys'

DELETE A FUNCTION BODY

  intact delete app.py --lines 120:145

REGEX WITH CAPTURE GROUPS

  intact replace app.py --regex --all \\
      --find 'def (\\w+)\\(' --with 'def test_$1('

$1 and ${name} expand in the replacement; --no-expand disables that. Rust regex
syntax; there are no backreferences or lookaround.

PREVIEW, THEN APPLY

  intact --dry-run replace app.py --find x --with y --all
  intact replace app.py --find x --with y --all

EDIT A LEGACY-ENCODED FILE WITH A KNOWN ENCODING

  intact --encoding windows-1252 replace legacy.txt --find 'mundo' --with 'mundão'

MIGRATE A FILE TO UTF-8, THEN EDIT FREELY

  intact convert legacy.txt --to utf-8
  intact replace legacy.txt --find 'mundo' --with '世界'

SEVERAL EDITS, ONE WRITE

  intact batch app.py --script - <<'EOF'
  [{\"op\":\"replace\",\"find\":\"DEBUG = True\",\"with\":\"DEBUG = False\"},
   {\"op\":\"append\",\"text\":\"# checked\"}]
  EOF

SCRIPTING AGAINST THE EXIT CODE

  intact replace app.py --find x --with y
  case $? in
    0) echo done ;;
    3) echo 'not there' ;;
    4) echo 'ambiguous, narrow the search' ;;
    5) echo 'encoding problem, run intact info' ;;
  esac",
    },
];

/// Generate a Markdown section for another project's agent instructions file
/// (CLAUDE.md, AGENTS.md, ...). `cmd` is how the binary should be invoked there.
/// What the generated section should say.
pub struct InstructionsSpec<'a> {
    pub cmd: &'a str,
    pub brief: bool,
    pub legacy_only: bool,
    pub heading_level: u8,
    /// The project mandates this encoding for every file.
    pub encoding: Option<&'a str>,
    /// The project mandates this line-ending style for every file.
    pub eol: Option<&'a str>,
    /// The agent's shell is a Windows one and the binary lives in WSL, so every
    /// command has to be launched through this (`wsl.exe`, `wsl.exe -d Ubuntu`).
    pub wsl: Option<&'a str>,
}

pub fn instructions(spec: &InstructionsSpec<'_>) -> String {
    let InstructionsSpec {
        cmd,
        brief,
        legacy_only,
        heading_level,
        encoding,
        eol,
        wsl,
    } = *spec;
    let mandated_encoding = encoding;
    let h = "#".repeat(heading_level.clamp(1, 4) as usize);
    let sub = format!("{h}#");

    // The commands below are written without the launcher prefix: repeating
    // `wsl.exe ` on thirty example lines costs more in readability than the one
    // rule up front is worth.
    let wsl_rule = match wsl {
        None => String::new(),
        Some(launcher) => format!(
            "\n{sub} Every command starts with `{launcher}`\n\n\
             `{cmd}` is a Linux program installed in WSL, and your shell is a Windows one \
             (PowerShell, cmd or Git Bash), which cannot execute it. Every command in this \
             section is written as `{cmd} ...` for readability; what you actually run is \
             `{launcher} {cmd} ...`:\n\n\
             ```bash\n\
             {launcher} {cmd} info FILE\n\
             {launcher} {cmd} replace FILE --find TEXT --with TEXT\n\
             ```\n\n\
             - **Paths are WSL paths.** `{launcher}` starts in the WSL view of the current \
             directory, so paths relative to it work as written — prefer them. A Windows \
             absolute path means nothing on the other side: `C:\\src\\app.py` arrives as one \
             long relative filename, so a read fails with exit 8 and a write creates a junk \
             file rather than editing the one you meant. Convert a path you were given with \
             `{launcher} wslpath 'C:\\src\\app.py'`, which prints `/mnt/c/src/app.py`.\n\
             - **\"Not found\" is a PATH problem, not a missing tool.** `{launcher} {cmd} \
             --version` prints a version when the setup is sound. `command not found` usually \
             means the binary is in `~/.local/bin`, which WSL does not put on PATH for \
             non-interactive commands — call it by its absolute WSL path instead. Never fall \
             back to another editing tool because this one would not start; say that it did \
             not start.\n\
             - **Quoting is your Windows shell's job.** In PowerShell prefer single quotes: \
             inside double quotes `$name` expands and a backtick escapes. When the text \
             contains quotes, `$`, or newlines, do not fight the shell — write it to a file in \
             the repository and pass `--text-file ./that-file`, or use `--escapes` and `\\n` in \
             a single-quoted argument.\n"
        ),
    };

    let eol_policy = match eol {
        None => String::new(),
        Some(style) => format!(
            "\n{sub} Line endings: always {upper}\n\n\
             Every file in this repository uses {upper} line endings, and every file you create \
             must too:\n\n\
             ```bash\n\
             # inserted text gets {upper}, and new files are written with it\n\
             export INTACT_EOL={style}\n\
             # and refuse to touch a file that is not already {upper}\n\
             export INTACT_STRICT_EOL=1\n\
             ```\n\n\
             Otherwise pass `--eol {style}` on every command. Without a mandate `{cmd}` matches \
             whatever the file already uses, which is right for a mixed repository and wrong \
             here.\n\n\
             With `INTACT_STRICT_EOL=1`, editing a file whose endings differ fails with exit \
             5 rather than leaving it with mixed endings. That is a pre-existing defect in the \
             file, not something your edit caused: normalise it in its own step with \
             `{cmd} convert FILE --newlines {style}`, and keep that separate from the change you \
             were asked to make so the diff stays readable.\n",
            upper = style.to_uppercase()
        ),
    };

    // A project that mandates one encoding must never fall back to detection:
    // a wrong single-byte guess writes wrong bytes rather than failing.
    let mandate = match mandated_encoding {
        None => String::new(),
        Some(label) => format!(
            "\n{sub} Encoding: always `{label}`\n\n\
             Every file in this repository is `{label}`, and every file you create must be too. \
             Never let `{cmd}` guess:\n\n\
             ```bash\n\
             # every invocation uses this encoding; detection never runs\n\
             export INTACT_ENCODING={label}\n\
             # and refuse to write at all if it ever falls back to guessing\n\
             export INTACT_NO_GUESS=1\n\
             ```\n\n\
             If those are not already set in your environment, pass `--encoding {label}` on \
             **every** `{cmd}` command instead — including `create`, which otherwise makes \
             UTF-8 files. `{cmd} info FILE` reports `detected_by: environment` or `explicit` \
             when the encoding was declared, and `guessed` when it was not; `guessed` on a write \
             is a bug in your invocation, not a detail to ignore.\n\n\
             Text you pass is still UTF-8 — `{cmd}` transcodes it into `{label}` for you. If a \
             character you need cannot be represented in `{label}`, the command fails with exit \
             5; that is a real conflict with the project's encoding policy, so raise it rather \
             than working around it with `--unmappable`.\n"
        ),
    };

    let (title, intro) = if legacy_only {
        (
            format!("{h} Editing non-UTF-8 files: use `{cmd}`\n"),
            format!(
                "Files in this repository are not all UTF-8. **Every edit to a file that is not \
                 UTF-8 must be made with `{cmd}`** — never with the built-in edit/write tools, \
                 `sed`, `awk`, `python`, or shell redirection.\n\n\
                 Reading a windows-1252 or Shift_JIS file, changing it, and writing it back \
                 re-encodes the whole file as UTF-8 and corrupts every non-ASCII character. \
                 `{cmd}` transcodes your UTF-8 text into the file's existing encoding and leaves \
                 every other byte byte-for-byte identical.\n\n\
                 If you are unsure whether a file is UTF-8, run `{cmd} info FILE` — or just use \
                 `{cmd}`, which is correct for UTF-8 files too."
            ),
        )
    } else {
        (
            format!("{h} File edits: `{cmd}` only\n"),
            format!(
                "**`{cmd}` is the only tool permitted to write to a file's contents in this \
                 repository.** Do not use the built-in edit/write tools, `sed`, `awk`, `perl`, \
                 `python`, `tee`, or `>`/`>>` redirection to change a file, and do not rewrite a \
                 file wholesale from memory. This covers creating, editing and truncating \
                 files; deleting, renaming and moving them is outside its scope, so use the \
                 normal tools (`git mv`, `rm`) for that.\n\n\
                 This is not a style preference. Files here are not all UTF-8, and reading a \
                 windows-1252 or Shift_JIS file, changing it, and writing it back re-encodes the \
                 whole file as UTF-8 — silently corrupting every non-ASCII character in it. \
                 `{cmd}` transcodes your UTF-8 text into the file's existing encoding, preserves \
                 its line endings and BOM, leaves every untouched byte identical, and writes \
                 atomically."
            ),
        )
    };

    if brief {
        let mut policy = String::new();
        if let Some(launcher) = wsl {
            policy.push_str(&format!(
                "\n`{cmd}` is a Linux binary in WSL and your shell is a Windows one: prefix \
                 every command below with `{launcher}`, and use paths relative to the current \
                 directory — a Windows path like `C:\\src\\app.py` does not exist inside WSL \
                 (`{launcher} wslpath 'C:\\src\\app.py'` converts one).\n"
            ));
        }
        if let Some(label) = mandated_encoding {
            policy.push_str(&format!(
                "\nEvery file here is `{label}`. Set `INTACT_ENCODING={label}` and \
                 `INTACT_NO_GUESS=1`, or pass `--encoding {label}` on every command — \
                 including `create`.\n"
            ));
        }
        if let Some(style) = eol {
            policy.push_str(&format!(
                "\nEvery file here uses {} line endings. Set `INTACT_EOL={style}` and \
                 `INTACT_STRICT_EOL=1`, or pass `--eol {style}` on every command. Fix a \
                 non-conforming file with `{cmd} convert FILE --newlines {style}`.\n",
                style.to_uppercase()
            ));
        }
        let encoding_line = policy;
        return format!(
            "{title}\n\
             {intro}\n\
             {encoding_line}\n\
             ```bash\n\
             {cmd} info FILE                                     # encoding, line endings, safety\n\
             {cmd} view FILE --lines 40:80 --number\n\
             {cmd} search FILE --find TEXT\n\
             {cmd} replace FILE --find TEXT --with TEXT          # anchor must be unique\n\
             {cmd} replace FILE --find TEXT --with TEXT --all\n\
             {cmd} insert FILE --line N --text TEXT\n\
             {cmd} delete FILE --lines 10:20\n\
             {cmd} replace-lines FILE --lines 5:7 --text TEXT\n\
             ```\n\n\
             Anchor each `replace` on text that occurs exactly once. A non-zero exit means \
             nothing was written: 3 no match, 4 ambiguous anchor, 5 encoding problem, 6 bad line \
             range. Never pass `--lossy` or `--force` unless the user asks for it. Run \
             `{cmd} guide` for the full manual and `{cmd} COMMAND --help` for one command.\n"
        );
    }

    format!(
        "{title}\n\
         {intro}\n\n\
         Run `{cmd} guide` for the complete manual, `{cmd} guide recipes` for worked examples, \
         or `{cmd} COMMAND --help` for a single command. Text you pass is always UTF-8.\n\
         {wsl_rule}{mandate}{eol_policy}\n\
         {sub} Commands\n\n\
         ```bash\n\
         # Inspect (never writes)\n\
         {cmd} info FILE                     # encoding, BOM, line endings, edit safety\n\
         {cmd} view FILE --lines 40:80 --number\n\
         {cmd} search FILE --find TEXT [--regex] [--ignore-case] [--lines 40:80]\n\n\
         # Edit\n\
         {cmd} replace FILE --find TEXT --with TEXT     # anchor must be unique\n\
         {cmd} replace FILE --find TEXT --with TEXT --all\n\
         {cmd} replace FILE --regex --find 'def (\\w+)\\(' --with 'def test_$1(' --all\n\
         {cmd} insert FILE --line N --text TEXT         # --after N to insert below\n\
         {cmd} append FILE --text TEXT\n\
         {cmd} prepend FILE --text TEXT\n\
         {cmd} delete FILE --lines 10:20\n\
         {cmd} replace-lines FILE --lines 5:7 --text-file /tmp/block.txt\n\
         {cmd} write FILE --text TEXT                   # replace whole contents\n\
         {cmd} create FILE --text TEXT [--parents]      # fails if the file exists\n\
         {cmd} convert FILE --to utf-8                  # migrate the encoding\n\
         {cmd} batch FILE --script ops.json             # many edits, one atomic write\n\
         ```\n\n\
         Text can come from `--text`, `--text-file PATH` or `--text-stdin` (for `replace`: \
         `--with`, `--with-file`, `--with-stdin`, or `--delete` to remove the match). Add \
         `--escapes` to write `\\n` inside a single `--text` argument. Line ranges are 1-based \
         and inclusive: `7`, `5:9`, `5:`, `:9`, `$`, `3:$`, `-3:-1`.\n\n\
         {sub} How to make an edit\n\n\
         1. Read the region you are about to change: `{cmd} view FILE --lines A:B --number`.\n\
         2. Choose an anchor that occurs exactly once in the file, and include enough \
         surrounding text to make it unique. `{cmd} search FILE --find TEXT` reports how many \
         times it occurs.\n\
         3. Apply the edit. Use `{cmd} batch` when a single file needs several changes, so they \
         land in one atomic write. For the same change across several files, run one command per \
         file — there is no glob or multi-file mode.\n\n\
         {sub} Rules\n\n\
         - **Anchor on unique text.** `replace` refuses an ambiguous anchor (exit 4) instead of \
         guessing. Extend `--find` until it is unique; reach for `--all` only when you actually \
         intend to change every occurrence, and `--occurrence N` only when position is the thing \
         you mean.\n\
         - **A non-zero exit means nothing was written.** 2 bad arguments, 3 no match, 4 \
         ambiguous anchor, 5 encoding problem, 6 line out of range, 7 file exists, 8 file not \
         found. Read the message and fix the cause; do not retry the same command hoping for a \
         different result. `--json` gives a parseable result on stdout and a \
         `{{\"ok\":false,\"kind\":...}}` object on stderr.\n\
         - **Never pass `--lossy` or `--force` on your own initiative.** They exist for a human \
         who has decided to accept the damage. `--lossy` rewrites the whole file and can change \
         bytes you never touched.\n\
         - **Exit 5 means stop and look.** Either the file's encoding cannot represent a \
         character you are inserting — run `{cmd} convert FILE --to utf-8` first if converting \
         the file is acceptable — or detection guessed the encoding wrong, in which case run \
         `{cmd} info FILE` and pass the right `--encoding LABEL`.\n\
         - **Preview with `--dry-run`** when an edit is large or you are unsure of its extent. \
         It prints a diff and writes nothing.\n\
         - **Do not \"fix\" mojibake by hand.** Characters like `Ã©` or `â€™` in a file mean an \
         earlier write used the wrong encoding. Report it rather than editing the damaged text \
         into a different shape.\n"
    )
}

pub fn find(topic: &str) -> Option<&'static Section> {
    let needle = topic.trim().to_ascii_lowercase();
    SECTIONS.iter().find(|s| s.key == needle)
}

pub fn topic_list() -> String {
    let width = SECTIONS.iter().map(|s| s.key.len()).max().unwrap_or(0);
    let mut out = String::from("Manual topics (`intact guide TOPIC`):\n\n");
    for section in SECTIONS {
        out.push_str(&format!("  {:<width$}  {}\n", section.key, section.summary));
    }
    out.push_str("\n`intact guide` with no topic prints all of them.\n");
    out
}

/// Each heading carries the topic key, so a reader of the full manual knows how
/// to ask for that one section again.
fn heading(section: &Section) -> String {
    let line = format!("{}  (intact guide {})", section.title, section.key);
    format!("{line}\n{}\n", "-".repeat(line.len()))
}

pub fn render_all() -> String {
    let mut out = String::from("intact manual\n===============\n");
    for section in SECTIONS {
        out.push_str(&format!("\n\n{}\n", heading(section)));
        out.push_str(section.body);
        out.push('\n');
    }
    out.push_str("\n\nSee also: `intact COMMAND --help` for a single command, and\n");
    out.push_str("`intact instructions` for a section to paste into a project's CLAUDE.md.\n");
    out
}

pub fn render_one(section: &Section) -> String {
    format!("{}\n{}\n", heading(section), section.body)
}
