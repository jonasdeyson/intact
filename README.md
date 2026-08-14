# intact

An encoding-preserving command-line text editor.

Some AI agents decode files to UTF-8 internally, so writing them back naively
re-encodes everything as UTF-8 and turns a Latin-1 `café` into `cafÃ©`.
`intact` takes UTF-8 text on the command line, transcodes it into whatever
encoding the target file already uses, and splices it in without touching any other byte.

```console
$ intact info cfg.ini
path:            cfg.ini
bytes:           137
encoding:        windows-1252 (detected by: guessed)
...
edit safety:     byte-exact (edits keep every untouched byte)

$ intact replace cfg.ini --find 'pequenas empresas' --with 'organizações públicas'
cfg.ini: updated (windows-1252, lf) - replaced 1 of 1 occurrence(s)
```

The replacement went in as `6f7267616e697a61e7f56573 20 fa62...` — `ç` as the
single byte `0xE7`, `õ` as `0xF5`, `ú` as `0xFA`. The file is still
windows-1252, and every byte outside the replaced span is untouched.

## Install

```console
cargo build --release
install -m755 target/release/intact ~/.local/bin/
```

## The binary documents itself

You can hand an agent nothing but the executable. Everything below is reachable
from `--help`:

```console
intact --help              # commands, global flags, exit codes, examples
intact COMMAND --help      # one command, with examples of its own
intact help COMMAND        # the same
intact guide               # the complete manual
intact guide --list        # its topics
intact guide recipes       # one topic
intact encodings           # supported encoding labels
```

`intact guide` covers the safety model, encodings and detection, line-range
syntax, text input and escapes, exit codes, JSON output, batch scripts, and
worked recipes. `--json` works on `guide` too, so the manual can be pulled in
structured form.

### Telling another project's agent about it

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
| `--wsl [DISTRO]` | the agent is on Windows and the binary is in WSL |

`--wsl` is for the split setup: `intact` built inside WSL as a Linux binary,
while the agent editing the project runs on Windows and types into PowerShell,
cmd or Git Bash. Running `intact` there fails — the ELF binary is not
executable by Windows. The generated section opens with the rule that every
command goes through `wsl.exe` (`--wsl Ubuntu-24.04` pins the distribution),
and covers the two follow-on traps: paths are WSL paths, so `C:\src\app.py` is
a junk relative filename on the other side, and quoting is the Windows shell's
job. The command list itself stays unprefixed and readable.

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

### Inspect

```console
intact info FILE                    # encoding, BOM, line endings, edit safety
intact view FILE [--lines 40:80] [--number]
intact search FILE --find TEXT [--regex] [--ignore-case] [--lines RANGE] [--max N]
intact encodings                    # supported encoding labels
intact guide [TOPIC]                # the built-in manual
intact instructions                 # a CLAUDE.md section for another project
```

`info` is worth running first on anything unfamiliar — it reports
`edit safety`, i.e. whether byte-exact editing is available.

`search` exits 3 when nothing matched (use `--allow-empty` for exit 0). Human
output is `path:line:column:line-text`.

### Edit

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
intact batch   FILE --script ops.json
```

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
| `5..9` | same as `5:9` |

`insert --line N` accepts one past the last line, meaning "start a new line at
the end".

#### Supplying text

Every command that takes text accepts one of:

| Flag | Source |
|---|---|
| `--text`, `-t` | the argument itself |
| `--text-file PATH` | a UTF-8 file |
| `--text-stdin` | standard input |

(`replace` uses `--find` / `--find-file` and `--with` / `--with-file` /
`--with-stdin` / `--delete`.)

Input text is **always UTF-8**. With `--escapes`, backslash sequences are
interpreted in `--text` and `--find`, which is the easy way to pass multi-line
content in a single argument:

```console
intact insert main.rs --line 1 --escapes --text 'use std::fmt;\nuse std::io;'
```

Supported escapes: `\n \r \t \0 \\ \' \" \xNN \uXXXX \u{XXXXX}`.

#### `batch` — several edits, one atomic write

Operations are applied in order; if any of them fails, **nothing is written**.
Later operations see the results of earlier ones, so line numbers refer to the
state at that step.

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

### Global options

| Option | Effect |
|---|---|
| `--json` | machine-readable result on stdout (errors on stderr) |
| `--dry-run`, `-n` | show a diff of what would change; write nothing |
| `--backup` | copy the original to `FILE.bak` first |
| `--encoding LABEL`, `-e` | force the file's encoding instead of detecting it |
| `--no-guess` | refuse to *write* to a file whose encoding was only guessed |
| `--unmappable POLICY` | `error` (default), `replace` (`?`), `xml` (`&#NNN;`), `skip` |
| `--eol MODE` | `auto` (default), `lf`, `crlf`, `cr`, `keep` |
| `--escapes` | interpret backslash escapes in supplied text |
| `--lossy` | permit rewriting a file that does not round-trip |
| `--force` | edit a file that contains NUL bytes |
| `--quiet`, `-q` | suppress the summary line |

## Encodings

Labels follow the [WHATWG Encoding Standard](https://encoding.spec.whatwg.org/):
`utf-8`, `utf-16le`, `utf-16be`, `windows-1250`…`windows-1258`, `iso-8859-2`…`-16`,
`koi8-r`, `koi8-u`, `macintosh`, `ibm866`, `gbk`, `gb18030`, `big5`, `euc-jp`,
`shift_jis`, `iso-2022-jp`, `euc-kr` and their usual aliases. Run
`intact encodings` for the list.

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
don't rely on detection, and don't rely on remembering `--encoding` either:

```bash
export INTACT_ENCODING=latin1   # every invocation, including `create`
export INTACT_NO_GUESS=1        # refuse to write if it ever falls back to guessing
```

The same applies to line endings, which have the identical trap: `--eol auto`
follows each file rather than the policy, and a brand-new file always gets LF.

```bash
export INTACT_EOL=crlf          # inserted text and new files
export INTACT_STRICT_EOL=1      # refuse to touch a file that isn't already CRLF
```

| Variable | Effect |
|---|---|
| `INTACT_ENCODING` | encoding for every invocation, as if `--encoding` were passed |
| `INTACT_NO_GUESS` | same as `--no-guess` |
| `INTACT_EOL` | line endings for every invocation, as if `--eol` were passed |
| `INTACT_STRICT_EOL` | same as `--strict-eol` |

The corresponding flag always overrides the variable. With these set, `create`
makes Latin-1 CRLF files rather than UTF-8 LF ones, detection never runs, and
`intact info` reports `detected_by: environment`. A bad value in a variable is
an error naming the variable, on every command, not a silent fallback.

`--no-guess` and `--strict-eol` gate *writes* only — `info`, `view` and `search`
keep working, so a file that trips a guard can still be inspected. `--strict-eol`
turns what would have been a silently mixed-ending file into exit 5:

```console
$ intact append app.c --text '/* done */'
intact: refusing to write: app.c has 2 line ending(s) that are not CRLF (lf=2, crlf=0, cr=0)
hint: normalise it first: `intact convert app.c --newlines crlf`
```

`write`, `create` and `convert` are exempt, since they replace the whole content
regardless. `convert --newlines` accepts no `--to`, so line endings can be fixed
without restating the encoding.

`intact instructions --encoding latin1 --eol crlf` generates a CLAUDE.md
section stating both policies and how to hold to them.

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

## Development

```console
cargo test        # 13 unit + 33 end-to-end tests
cargo clippy --all-targets
```

The manual lives in [`src/manual.rs`](src/manual.rs), not in this README — the
binary is the source of truth, and tests assert that every command appears in
the top-level help and that every manual topic renders.
