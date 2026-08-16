# intact

[![CI](https://github.com/jonasdeyson/intact/actions/workflows/ci.yml/badge.svg)](https://github.com/jonasdeyson/intact/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/intact.svg)](https://crates.io/crates/intact)
[![MSRV](https://img.shields.io/badge/rust-1.85%2B-blue.svg)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

An encoding-preserving command-line text editor.

## Summary

It is a known issue that some AI agents corrupt files containing accented characters
(á, à, ã, ú, ç, etc.) when editing them.
This seems to happen especially when the files use an encoding other than UTF-8, and can affect characters far from the sections being edited.

`intact` is a command-line text editor that prevents character corruption by preserving encodings and EOL sequences, and by not touching bytes outside the edited segments.
This makes it a reliable text editing tool for AI agents prone to text corruption problems.

## Install

### Precompiled binaries

Precompiled binaries for Linux, macOS and Windows are available for each 
[release](https://github.com/jonasdeyson/intact/releases) — unpack the archive
for your platform and put the `intact` executable on `PATH`; no Rust toolchain
needed.

### crates.io

You can install from [crates.io](https://crates.io/crates/intact) (requires Rust toolchain):

```console
cargo install intact
```

### From source

Install directly from the source root (requires Rust toolchain):

```console
cargo install --path .
```

## Documentation

The full manual — safety model, encodings and detection, line-range syntax,
text input and escapes, exit codes, JSON output, batch scripts, worked
recipes — is **[MANUAL.md](MANUAL.md)**, generated straight from the binary so
it can't drift from what the tool actually does.

It's also built into the executable, so you can hand an agent nothing but the
binary and it can discover everything from `--help`:

```console
intact --help              # commands, global flags, exit codes, examples
intact COMMAND --help      # one command, with examples of its own
intact guide               # the complete manual (same text as MANUAL.md)
intact guide --list        # its topics
intact guide encoding      # one topic, e.g. encoding, ranges, batch, recipes
```

`--json` works on `guide` too, so the manual can be pulled in structured form.

Everything is configured by flags — `intact` reads no environment variables and
no configuration file, since an agent typically runs each command in a fresh
shell where an `export` wouldn't survive to the next invocation.

### Teaching an agent how to use it

`intact instructions` prints a Markdown section to paste into another
project's `CLAUDE.md` / `AGENTS.md`:

```console
intact instructions >> ../some-project/CLAUDE.md
intact instructions --brief >> ../some-project/AGENTS.md
```

The generated section states that `intact` is the only tool permitted to
modify files in that repository, explains why, lists the commands, and gives
rules for anchoring edits and reading exit codes. Options:

| Option | Effect |
|---|---|
| `--brief` | a handful of lines instead of a full section |
| `--command NAME` | how the binary is invoked there (e.g. an absolute path) |
| `--legacy-only` | narrow the mandate to non-UTF-8 files only |
| `--heading-level N` | heading depth, 1–4 (default 2) |
| `--wsl [DISTRO]` | can be used when the agent is on Windows and `intact` is a Linux binary in WSL |

## Guarantees

1. **Encoding is preserved.** The file's encoding is detected (BOM → valid
   UTF-8 → statistical detection) or forced with `--encoding`. Output is written
   in that same encoding.
2. **Untouched bytes stay identical.** When the file round-trips through its
   encoding, edits are applied by splicing encoded bytes into the original
   buffer. Everything outside the edited region is the byte that was already
   there, so repeated edits cannot accumulate mojibake.
3. **No silent corruption.** If the file does not round-trip, or if your text
   contains a character the file's encoding cannot represent, the command
   *fails* with a distinct exit code and writes nothing. You opt into lossy
   behaviour explicitly (`--lossy`, `--unmappable`).
4. **Line endings and BOMs are preserved.** Inserted text is rewritten to the
   file's dominant line ending; a BOM stays exactly as it was.
5. **Writes are atomic.** Content goes to a temporary file in the same
   directory, then is renamed over the original, carrying its permissions.
   Editing a symlink writes the file it points at and leaves the link intact;
   the result line names both (`link.txt -> real.txt: updated ...`) and `--json`
   adds a `resolved_path` field, so an edit landing somewhere other than the
   path you typed is never silent.

## Commands

### Inspection

```console
intact info FILE                    # encoding, BOM, line endings, edit safety
intact view FILE [--lines 40:80] [--number]
intact search FILE --find TEXT [--regex] [--ignore-case] [--lines RANGE] [--max N]
intact guide [TOPIC]                # the built-in manual
intact instructions                 # a CLAUDE.md section for another project
```

`info` is worth running first on anything unfamiliar — it reports
`edit safety`, i.e. whether byte-exact editing is available.

`search` exits 3 when nothing matched (use `--allow-empty` for exit 0). Human
output is `path:line:column:line-text`.

### Editing

```console
intact replace FILE --find TEXT --with TEXT
intact insert  FILE --line N   --text TEXT      # insert before line N
intact insert  FILE --after N  --text TEXT      # insert after line N
intact append  FILE --text TEXT
intact prepend FILE --text TEXT
intact delete  FILE --lines 10:20
intact replace-lines FILE --lines 5:7 --text TEXT
intact move-lines FILE --lines 40:52 --after 12  # --before N, or --by -3
intact write   FILE --text TEXT                 # replace whole contents
intact create  FILE --text TEXT                 # fails if the file exists
intact convert FILE --to utf-8
intact batch   FILE --script ops.json           # several edits, or several files
```

**When a file needs more than one edit, reach for `batch`** rather than a run of
separate commands: one JSON script of the same ops above, applied in order,
written once, and nothing written at all if any operation fails. It's also the
only command that can edit more than one file in one call. `replace`'s search
text must be unique by default (exit 4 if it matches more than once, exit 3 if
it matches none) — the same safety property agent edit tools generally have.
Details, flags and worked examples for every command, including `move-lines`,
line-range syntax, escapes, and the full `batch` script format, are in
[MANUAL.md](MANUAL.md).

### Global options

| Option | Effect |
|---|---|
| `--json` | machine-readable result on stdout (errors on stderr) |
| `--dry-run`, `-n` | print a unified diff of what would change; write nothing |
| `--show-diff` | print a unified diff of the change *and* apply it |
| `--diff-context N` | unchanged lines shown either side of a change (default 3) |
| `--backup` | copy the original to `FILE.bak` first |
| `--encoding LABEL`, `-e` | force the file's encoding instead of detecting it |
| `--no-guess` | refuse to *write* to a file whose encoding was only guessed |
| `--unmappable POLICY` | `error` (default), `replace` (`?`), `xml` (`&#NNN;`), `skip` |
| `--eol MODE` | `auto` (default), `lf`, `crlf`, `cr`, `keep` |
| `--strict-eol` | refuse to write when the file's existing line endings don't match `--eol` |
| `--escapes` | interpret backslash escapes in supplied text |
| `--lossy` | permit rewriting a file that does not round-trip |
| `--force` | edit a file that contains NUL bytes |
| `--quiet`, `-q` | suppress the summary line |

## Seeing the change

`--show-diff` applies an edit *and* prints a unified diff of what it changed —
useful because an agent harness typically shows the raw command line at
approval time, before `intact` has run, so this is how the change itself gets
seen. `--dry-run` prints the same diff without writing. Both, plus how `--json`
reports edits structurally, are covered in [MANUAL.md](MANUAL.md).

## Encodings

Labels follow the [WHATWG Encoding Standard](https://encoding.spec.whatwg.org/):
`utf-8`, `utf-16le`/`be`, `windows-1250`…`1258`, `iso-8859-2`…`16`, `koi8-r`,
`koi8-u`, `macintosh`, `ibm866`, `gbk`, `gb18030`, `big5`, `euc-jp`,
`shift_jis`, `iso-2022-jp`, `euc-kr` and their usual aliases — run `intact
guide encoding` for the complete list this build accepts. What happens when a
file doesn't round-trip, when text can't be represented, or when detection
guesses wrong is covered in the `ENCODING` section of [MANUAL.md](MANUAL.md)
(same content as `intact guide encoding`).
