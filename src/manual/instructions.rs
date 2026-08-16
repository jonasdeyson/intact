//! Generate a Markdown section for another project's agent instructions file
//! (CLAUDE.md, AGENTS.md, ...), served by `intact instructions`.

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
             must too. Pass both flags on **every** command that writes:\n\n\
             ```bash\n\
             {cmd} --eol {style} --strict-eol replace FILE --find TEXT --with TEXT\n\
             ```\n\n\
             `--eol {style}` gives inserted text and new files the right terminators; without it \
             `{cmd}` matches whatever the file already uses, which is right for a mixed \
             repository and wrong here.\n\n\
             `--strict-eol` makes editing a file whose endings differ fail with exit 5 rather \
             than leaving it with mixed endings. That is a pre-existing defect in the file, not \
             something your edit caused: normalise it in its own step with \
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
             Never let `{cmd}` guess — pass both flags on **every** command:\n\n\
             ```bash\n\
             {cmd} --encoding {label} --no-guess replace FILE --find TEXT --with TEXT\n\
             ```\n\n\
             `--encoding {label}` applies to `create` too, which otherwise makes UTF-8 files. \
             `--no-guess` turns a fall-back to statistical detection into an error instead of a \
             silent wrong guess.\n\n\
             `{cmd} info FILE` reports `detected_by: explicit` when the encoding was declared. \
             On a write, anything else means the flag did not reach the command. Do not read \
             `utf-8-valid` as reassurance: it says only that the bytes *can* be read as UTF-8, \
             which is also true of plenty of real `{label}` files, and `--no-guess` does not \
             catch it — that guard covers `guessed` alone. `--encoding {label}` is the only \
             thing that makes the encoding a decision rather than an inference.\n\n\
             Put the flags in the command every time. Do not try to set this once for the \
             session — `{cmd}` reads no environment variables and no config file, and each of \
             your commands may run in a fresh shell anyway.\n\n\
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
                "\nEvery file here is `{label}`. Pass `--encoding {label} --no-guess` on every \
                 command — including `create`.\n"
            ));
        }
        if let Some(style) = eol {
            policy.push_str(&format!(
                "\nEvery file here uses {} line endings. Pass `--eol {style} --strict-eol` on \
                 every command. Fix a non-conforming file with \
                 `{cmd} convert FILE --newlines {style}`.\n",
                style.to_uppercase()
            ));
        }
        if mandated_encoding.is_some() || eol.is_some() {
            policy.push_str(
                "\nPass those as flags every time. `intact` reads no environment variables and \
                 no config file, and each command you run may start a fresh shell, so there is \
                 nothing to set once.\n",
            );
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
             {cmd} move-lines FILE --lines 40:52 --after 12      # relocate a block\n\
             {cmd} batch FILE --script ops.json                  # several edits at once\n\
             ```\n\n\
             **When a file needs more than one change, use `batch`** rather than a run of \
             separate commands. It takes a JSON script of operations, applies them in order, and \
             writes once; if any operation fails, nothing is written at all. Give an operation \
             its own `\"file\"` to edit several files in the same command — that is the only \
             multi-file mode.\n\n\
             ```bash\n\
             {cmd} batch app.py --script - <<'EOF'\n\
             [{{\"op\":\"replace\",\"find\":\"a\",\"with\":\"b\"}},\n\
              {{\"op\":\"replace\",\"file\":\"other.py\",\"find\":\"c\",\"with\":\"d\"}}]\n\
             EOF\n\
             ```\n\n\
             Anchor each `replace` on text that occurs exactly once. A non-zero exit means \
             nothing was written: 3 no match, 4 ambiguous anchor, 5 encoding problem, 6 bad line \
             range. Never pass `--lossy` or `--force` unless the user asks for it.\n\n\
             Add `--show-diff` to every edit — it applies the edit and prints a unified diff of \
             what changed, which is the only way the change is visible to whoever reads your \
             output. Use `--dry-run` in its place to preview without writing. Put global flags \
             before the subcommand: `{cmd} --show-diff replace FILE ...`.\n\n\
             Run `{cmd} guide` for the full manual and `{cmd} COMMAND --help` for one command.\n"
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
         {cmd} move-lines FILE --lines 40:52 --after 12  # --before N, or --by -3\n\
         {cmd} write FILE --text TEXT                   # replace whole contents\n\
         {cmd} create FILE --text TEXT [--parents]      # fails if the file exists\n\
         {cmd} convert FILE --to utf-8                  # migrate the encoding\n\
         {cmd} batch FILE --script ops.json             # several edits, and several files\n\
         ```\n\n\
         Text can come from `--text` or `--text-file PATH` (for `replace`: `--with`, \
         `--with-file`, or `--delete` to remove the match). `--text-file -` reads standard \
         input, as does any other path argument given `-`. Add `--escapes` to write `\\n` inside \
         a single `--text` argument. Line ranges are 1-based and inclusive: `7`, `5:9`, `5:`, \
         `:9`, `$`, `3:$`, `-3:-1`.\n\n\
         {sub} More than one edit: use `batch`\n\n\
         A file that needs several changes takes one `{cmd} batch`, not one command per change. \
         The script is a JSON list of operations applied in order, each seeing the result of the \
         last, written once at the end; if any operation fails, nothing is written at all. An \
         operation may name its own `\"file\"`, which is the only way to edit several files in \
         one command.\n\n\
         ```bash\n\
         {cmd} batch app.py --script - <<'EOF'\n\
         [{{\"op\":\"replace\",\"find\":\"DEBUG = True\",\"with\":\"DEBUG = False\"}},\n\
          {{\"op\":\"delete\",\"lines\":\"40:42\"}},\n\
          {{\"op\":\"append\",\"file\":\"CHANGELOG.md\",\"text\":\"- turned off debug\"}}]\n\
         EOF\n\
         ```\n\n\
         Ops: `replace` (find, with, regex, ignore_case, all, occurrence, expect, lines, \
         no_expand), `insert` (line or after, text), `append`, `prepend` (text), `delete` \
         (lines), `replace-lines` (lines, text), `move-lines` (lines, and one of after, before, \
         by), `write` (text) — plus `file` on any of them. \
         Run `{cmd} guide batch` for the full schema.\n\n\
         {sub} How to make an edit\n\n\
         1. Read the region you are about to change: `{cmd} view FILE --lines A:B --number`.\n\
         2. Choose an anchor that occurs exactly once in the file, and include enough \
         surrounding text to make it unique. `{cmd} search FILE --find TEXT` reports how many \
         times it occurs.\n\
         3. Apply the edit — a single `replace` for one change, `{cmd} batch` for several or for \
         more than one file.\n\n\
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
         bytes you never touched. `--force` overrides the refusal to touch a file that does not \
         read as text, for reading as well as editing: if `view` or `search` refuses a file, it \
         is not one you should be reading, and forcing it puts raw bytes into your context.\n\
         - **Exit 5 means stop and look.** Either the file's encoding cannot represent a \
         character you are inserting — run `{cmd} convert FILE --to utf-8` first if converting \
         the file is acceptable — or detection guessed the encoding wrong, in which case run \
         `{cmd} info FILE` and pass the right `--encoding LABEL`.\n\
         - **Show the change.** Add `--show-diff` to every edit: it applies the edit and prints \
         a unified diff of what it changed, so the person reading your output can see the change \
         without taking your word for it. A permission prompt for a `{cmd}` command shows the \
         command line, not a diff, and by the time `{cmd}` prints anything the edit is already \
         approved — `--show-diff` is what closes that gap. Use `--dry-run` instead when you are \
         deciding *whether* to make the edit: it prints the same diff and writes nothing. Prefer \
         one `--show-diff` over a `--dry-run` followed by the real command, which is two \
         approvals for one change and can drift between them.\n\
         - **Global flags go before the subcommand**, as in `{cmd} --dry-run replace FILE ...`, \
         not after it. Both orders work, but only the first can be matched by a prefix rule, \
         which is how a preview gets pre-approved while a real write still prompts.\n\
         - **Pass every setting as a flag.** `{cmd}` reads no environment variables and no \
         config file, so there is nothing to set once for a session — and if each of your \
         commands runs in a fresh shell, an `export` would not survive to the next one anyway.\n\
         - **Do not \"fix\" mojibake by hand.** Characters like `Ã©` or `â€™` in a file mean an \
         earlier write used the wrong encoding. `{cmd}` reports these as `mojibake` in `info` \
         and warns before writing to such a file. Report it rather than editing the damaged \
         text into a different shape.\n"
    )
}
