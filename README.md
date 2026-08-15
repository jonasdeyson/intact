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

## Self-contained documentation

You can hand an agent nothing but the executable. Everything below is reachable
from `--help`:

```console
intact --help              # commands, global flags, exit codes, examples
intact COMMAND --help      # one command, with examples of its own
intact help COMMAND        # the same
intact guide               # the complete manual
intact guide --list        # its topics
intact guide recipes       # one topic
intact guide encoding      # detection, labels, and the full label list
```

`intact guide` covers the safety model, encodings and detection, line-range
syntax, text input and escapes, exit codes, JSON output, batch scripts, and
worked recipes. `--json` works on `guide` too, so the manual can be pulled in
structured form.

Everything is configured by flags. `intact` reads no environment variables and
no configuration file — an agent typically runs each command in a fresh shell,
so an `export` from one invocation would not survive to the next, and a setting
that applies only sometimes is worse than one that never applies.

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

## The guarantees

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

## Exit codes

| Code | Meaning |
|-----:|---------|
| 0 | success |
| 1 | generic failure |
| 2 | bad arguments |
| 3 | no match / target not found |
| 4 | ambiguous match (more occurrences than allowed) |
| 5 | encoding problem (undecodable file, or text unrepresentable in it) |
| 6 | line number or range out of bounds |
| 7 | file already exists (`create`) |
| 8 | file not found |
| 9 | I/O error |

With `--json`, failures print a parseable object on **stderr**:

```json
{"ok":false,"error":"3 occurrences of text \"x\" (first at line 1, column 1); refusing to guess","kind":"ambiguous","hint":"pass --all to replace every occurrence, ...","exit_code":4}
```

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
intact write   FILE --text TEXT                 # replace whole contents
intact create  FILE --text TEXT                 # fails if the file exists
intact convert FILE --to utf-8
intact batch   FILE --script ops.json           # several edits, or several files
```

**When a file needs more than one edit, reach for `batch`** rather than a run of
separate commands: one JSON script, applied in order, written once, and nothing
written at all if any operation fails. It is also the only command that can edit
more than one file.

#### `replace` in detail

By default the search text **must be unique** — this is the safety property an
agent wants, and it mirrors how agent edit tools behave. Exit 4 if it matches
more than once, exit 3 if it matches nothing.

```console
intact replace app.py --find 'timeout = 30' --with 'timeout = 60'
intact replace app.py --find old_name --with new_name --all
intact replace app.py --find x --with y --occurrence 2      # only the 2nd
intact replace app.py --find x --with y --expect 3          # require exactly 3
intact replace app.py --find x --with y --lines 40:80       # only in that range
intact replace app.py --find 'dead_code()' --delete         # remove the match
intact replace app.py --regex --all \
    --find 'def (\w+)\(' --with 'def test_$1('                # $1 expands
```

`--regex` uses Rust `regex` syntax; `$1` / `${name}` expand in the replacement
unless `--no-expand` is given. Without `--regex` the needle is literal, and its
`\n` are rewritten to the file's line ending so a literal multi-line search
works on CRLF files too.

#### Line numbers and ranges

Everything is 1-based and inclusive. `--lines` accepts:

| Form | Meaning |
|---|---|
| `7` | line 7 |
| `5:9` | lines 5 through 9 |
| `5:` | line 5 to end of file |
| `:9` | start of file through line 9 |
| `$` or `end` | the last line |
| `3:$` | line 3 to the last line |
| `-3:-1` | the last three lines |

`:` is the only separator.

`insert --line N` accepts one past the last line, meaning "start a new line at
the end".

#### Supplying text

Every command that takes text accepts one of:

| Flag | Source |
|---|---|
| `--text`, `-t` | the argument itself |
| `--text-file PATH` | a UTF-8 file; `-` means standard input |

(`replace` uses `--find` / `--find-file` and `--with` / `--with-file` /
`--delete`.)

Every path argument that reads text takes `-` for standard input, `batch
--script -` included; there is no separate `--text-stdin` flag.

Input text is **always UTF-8**. With `--escapes`, backslash sequences are
interpreted in `--text`, `--find` and `--with`, which is the easy way to pass
multi-line content in a single argument:

```console
intact insert main.rs --line 1 --escapes --text 'use std::fmt;\nuse std::io;'
```

Supported escapes: `\n \r \t \0 \\ \' \" \xNN \uXXXX \u{XXXXX}`.

`--escapes` applies to the text whatever it came from, so `--text-file`,
`--find-file` and `--with-file` content is unescaped too — worth remembering
before pointing it at a block full of Windows paths. Text in a `batch` script is
never touched by it: JSON has escapes of its own.

#### `batch` — several edits, and the only multi-file mode

Operations are applied in order; if any of them fails, **nothing is written** —
not for that file, and not for any other. Later operations see the results of
earlier ones, so line numbers refer to the state at that step.

```json
{
  "ops": [
    { "op": "replace", "find": "DEBUG = True", "with": "DEBUG = False" },
    { "op": "replace", "find": "log(", "with": "logger.info(", "all": true },
    { "op": "delete", "lines": "40:42" },
    { "op": "insert", "line": 1, "text": "# generated" },
    { "op": "append", "text": "# end" }
  ]
}
```

A bare JSON array works too. Recognised ops: `replace` (`find`, `with`,
`regex`, `ignore_case`, `all`, `occurrence`, `expect`, `lines`, `no_expand`),
`insert` (`line` or `after`, `text`), `append`, `prepend`, `delete` (`lines`),
`replace-lines` (`lines`, `text`), `write` (`text`). Use `--script -` to read
the script from stdin.

Every op also takes a `file`, which is how one command edits several files. The
`FILE` argument is the default for ops that omit it, and can be left out
entirely when they all name one:

```console
$ intact batch --script - <<'EOF'
[{"op":"replace","file":"src/a.py","find":"old","with":"new","all":true},
 {"op":"replace","file":"legacy.txt","find":"old","with":"new","all":true},
 {"op":"append","file":"CHANGELOG.md","text":"- renamed old to new"}]
EOF
src/a.py: updated (UTF-8, lf) - applied 1 operation(s)
legacy.txt: updated (windows-1252, crlf) - applied 1 operation(s)
CHANGELOG.md: updated (UTF-8, lf) - applied 1 operation(s)
```

Each file is decoded, checked and written back in its own encoding, which is
why every other command takes exactly one file. Reporting is one summary line
per file; under `--json`, it's a `files` array with one object per file —
always an array, whether the script touched one file or twenty.

Every operation runs against an in-memory copy and nothing reaches disk until
all of them have succeeded. The writes are then one atomic rename per file;
`intact` cannot make a rename across several files atomic, so an I/O error
partway through that last step can leave earlier files written. A failing
*operation* never writes anything.

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

An agent's permission prompt shows the command line it is about to run. For a
built-in edit tool the harness can render a diff from the tool's arguments, but
a shell command is one opaque string — and by the time `intact` prints anything,
the edit is already approved. `--show-diff` closes that gap from the other side:
it applies the edit *and* prints a unified diff of what it changed.

```console
$ intact --show-diff replace app.py --find 'timeout = 30' --with 'timeout = 60'
--- app.py
+++ app.py
@@ -10,7 +10,7 @@
 def connect(host):
     sock = socket.create_connection((host, 443))
-    timeout = 30
+    timeout = 60
     sock.settimeout(timeout)
app.py: updated (UTF-8, lf) - replaced 1 of 1 occurrence(s)
```

Prefer that over `--dry-run` followed by the real command. Two invocations are
two approvals for one change, and the file can differ between them; one
invocation that reports exactly what it changed cannot drift. `--dry-run` is for
deciding *whether* to make the edit — it prints the same diff and writes
nothing.

For a project where every edit should show its work, `intact instructions`
generates a rule telling the other project's agent to pass `--show-diff` on
every edit.

The output is a real unified diff. `--quiet` suppresses only the summary line,
so stdout is the patch and nothing else:

```console
$ intact --dry-run --quiet replace app.py --find x --with y > change.patch
$ git apply -p0 --check change.patch
```

Line terminators are deliberately not compared line by line — a file converted
from LF to CRLF would otherwise report every line as changed. They are reported
above the diff instead, whenever the styles in use change:

```console
$ intact --dry-run --eol crlf append app.c --text '/* done */'
# line endings: lf=42 crlf=0 cr=0 -> lf=42 crlf=1 cr=0
--- app.c
+++ app.c
@@ -40,3 +40,4 @@
...
```

That line is how a CRLF line landing in an LF file becomes visible. A change
with no textual difference at all — re-encoding a file, adding a BOM — says so
rather than printing nothing.

### Pre-approving previews

Global flags are accepted before or after the subcommand, but write them
before it:

```console
intact --dry-run replace app.py --find x --with y      # do this
intact replace app.py --find x --with y --dry-run      # not this
```

Only the first form can be matched by a tool that allows commands by prefix.
Under an agent harness that asks permission per command, a rule matching
`intact --dry-run ` pre-approves every preview while leaving real writes to
prompt — which only works if the flag is where a prefix can see it. In Claude
Code that is a `Bash(intact --dry-run:*)` entry in `.claude/settings.json`.

### Reading the change from JSON

`--json` reports the spans that were replaced, which is what `intact` actually
did rather than what comparing two versions of the file suggests it did:

```json
"edit_count": 1,
"edits": [{"line": 12, "column": 5, "end_line": 12, "end_column": 17,
           "offset": 243, "end_offset": 255,
           "before": "timeout = 30", "after": "timeout = 60",
           "truncated": false}]
```

With `--dry-run` or `--show-diff` a `"diff"` field carries the complete unified
diff — never truncated, unlike the human output — plus `"eol_before"` and
`"eol_after"` when the line-ending styles change.

Two fields appear only when there is something to report, so their presence is
itself the signal: `"resolved_path"` when the path given is a symlink, and
`"warnings"` when something about the file deserves saying without blocking the
write.

## Encodings

Labels follow the [WHATWG Encoding Standard](https://encoding.spec.whatwg.org/):
`utf-8`, `utf-16le`, `utf-16be`, `windows-1250`…`windows-1258`, `iso-8859-2`…`-16`,
`koi8-r`, `koi8-u`, `macintosh`, `ibm866`, `gbk`, `gb18030`, `big5`, `euc-jp`,
`shift_jis`, `iso-2022-jp`, `euc-kr` and their usual aliases. `intact guide
encoding` ends with the complete list this build accepts.

Two things worth knowing:

- Per the standard, `latin1` / `iso-8859-1` resolve to **windows-1252**, which
  differs from strict ISO 8859-1 only in how bytes `0x80`–`0x9F` are named. Both
  round-trip, so the bytes on disk are unaffected either way.
- `iso-2022-jp` is stateful, so byte-exact splicing is unavailable for it; such
  files need `--lossy`, which re-encodes the whole file.

## When a command refuses

```console
$ intact replace legacy.txt --find foo --with bar
intact: refusing to edit: legacy.txt does not round-trip through windows-1252:
re-encoding the decoded text would change untouched bytes
hint: pass --encoding LABEL if the encoding was guessed wrong, run `intact info FILE`
to inspect, or pass --lossy to rewrite the whole file anyway
```

This means detection picked an encoding that cannot reproduce the original
bytes. Run `intact info` and pass the right `--encoding`. Reach for `--lossy`
only when you accept that untouched parts of the file may change.

```console
$ intact replace notes.txt --find mundo --with 世界
intact: character '世' (U+4E16) cannot be represented in windows-1252
hint: convert the file first (`intact convert FILE --to utf-8`) or pass
--unmappable replace|xml|skip
```

The file's encoding has no room for that character. Either convert the file to
UTF-8 first, or choose a substitution policy.

### Projects that mandate one encoding

If every file in a project must be Latin-1 (or Shift_JIS, or anything else),
don't rely on detection. Say so on every command that writes:

```console
$ intact --encoding latin1 --no-guess replace notes.txt --find mundo --with mundão
```

`--encoding` applies to `create` too, which otherwise makes UTF-8 files;
`--no-guess` turns a fall-back to statistical detection into exit 5 rather than
a silent wrong guess.

Line endings have the identical trap: `--eol auto` follows each file rather than
the policy, and a brand-new file always gets LF.

```console
$ intact --eol crlf --strict-eol append app.c --text '/* done */'
```

There is deliberately **no environment variable and no config file** for any of
this. `intact` is typically driven one command per shell — that is how an agent
runs it — so an `export` in one invocation is gone by the next, and a mandate
that holds only sometimes is worse than one that never holds. Put the flags in
the command. `intact instructions --encoding latin1 --eol crlf` generates a
CLAUDE.md section that tells the other project's agent to do exactly that.

`--no-guess` and `--strict-eol` gate *writes* only — `info`, `view` and `search`
keep working, so a file that trips a guard can still be inspected. `--strict-eol`
turns what would have been a silently mixed-ending file into exit 5:

```console
$ intact --eol crlf --strict-eol append app.c --text '/* done */'
intact: refusing to write: app.c has 2 line ending(s) that are not CRLF (lf=2, crlf=0, cr=0)
hint: normalise it first: `intact convert app.c --newlines crlf`
```

`write`, `create` and `convert` are exempt, since they replace the whole content
regardless. `convert --newlines` accepts no `--to`, so line endings can be fixed
without restating the encoding.

### A note on guessed encodings

Statistical detection needs a reasonable amount of text. On a short file with
only one or two non-ASCII bytes it may land on the wrong single-byte encoding —
`windows-1250` instead of `windows-1252`, say. Existing bytes are still safe
(every single-byte encoding round-trips, so nothing already in the file is
rewritten), but the *character repertoire* differs, and inserting `ã` into a
file believed to be windows-1250 fails:

```console
$ intact replace notes.txt --find mundo --with mundão
intact: character 'ã' (U+00E3) cannot be represented in windows-1250
(this file's encoding was guessed, not declared)
hint: if windows-1250 is not really the file's encoding, pass --encoding LABEL;
otherwise convert the file (`intact convert FILE --to utf-8`) or pass
--unmappable replace|xml|skip

$ intact --encoding windows-1252 replace notes.txt --find mundo --with mundão
notes.txt: updated (windows-1252, lf) - replaced 1 of 1 occurrence(s)
```

`intact info` always reports whether the encoding was declared by a BOM,
proven by valid UTF-8, or merely guessed. When a project's encoding is known,
passing `--encoding` removes the guesswork entirely.

### Mojibake already in the file

Text that has been through the wrong encoding leaves a recognisable shape — `Ã©`
where `é` was meant, `â€™` where a right single quote was. `intact info` reports
it as a `warning:` line, and every command that writes prints the same warning on
stderr before its result:

```console
$ intact replace notes.txt --find addition --with note
intact: warning: notes.txt: 2 mojibake-shaped sequence(s), first "Ã©" at line 1:
text that was written through the wrong encoding at some point. Report it rather
than editing the damaged text by hand. This file's encoding was inferred
(utf-8-valid), not declared - confirm it with --encoding LABEL before writing.
notes.txt: updated (UTF-8, lf) - replaced 1 of 1 occurrence(s)
```

It is an advisory, not a guard: the edit goes through, and `--quiet` does not
silence it. Under `--json` it is a `mojibake` object on an `info` result
(`count`, `line`, `sample`) and a `warnings` array on an edit result.

The check reports damage, not misdetection. It catches text that was written
through the wrong encoding at some point — including UTF-8 that was
double-encoded — but not a file whose encoding `intact` read wrong: a
windows-1252 file whose bytes are valid UTF-8 decodes to *clean* text, and only
`--encoding` settles that case. Don't hand-fix the characters either. Rewriting
one `Ã©` as `é` repairs a single occurrence and leaves the rest of the file as it
was; the repair is to re-encode the whole file from the encoding it was mangled
through.

## Development

```console
cargo test        # unit + end-to-end tests
cargo clippy --all-targets
cargo fmt --check
```

CI runs those three on every push and pull request — the test suite on Linux,
macOS and Windows, plus a `cargo check` against the minimum supported Rust
version (1.85, the edition 2024 floor). The workflow is
[`.github/workflows/ci.yml`](.github/workflows/ci.yml).

The manual lives in [`src/manual.rs`](src/manual.rs), not in this README — the
binary is the source of truth, and tests assert that every command appears in
the top-level help and that every manual topic renders.
