# Changelog

All notable changes to this project are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

[Unreleased]

### Changed

- The binary guard now covers reading as well as writing. `view`, `search` and
  `convert` refuse a file that does not read as text, where before only the
  writing commands did and `view` would emit raw NULs and escape sequences into
  the terminal and exit 0. `--force` is still the single override and now
  applies to reads too. `info` is the exception and always reports, since it is
  how a caller learns why the others refused.
- The check runs before the file is decoded, so a large blob is no longer
  decoded and indexed on its way to being refused.
- Detection is no longer "contains a NUL". The first 8 KiB are judged on a NUL
  byte, on the proportion of stray control characters — which catches
  high-entropy data carrying no NUL, previously accepted outright — and, for
  UTF-16, on surrogate pairing. UTF-16 was formerly exempted wholesale, so a
  binary file beginning `FF FE` skipped the guard entirely.
- Refusals name the reason: `NUL byte at offset 7 (0x7)`, `10% control
  characters (204 of the first 2000)`, `unpaired UTF-16 surrogate at offset 112`.
- `info` on a non-text file reports its bytes and a verdict, and withholds the
  text-level report entirely. Everything it used to print below `bytes` —
  `encoding`, `bom`, `line endings`, `lines`, `characters`, `final newline`,
  `edit safety` — describes the file decoded as text, and none of it is a fact
  about a file that is not text: the encoding is a guess about a blob, the line
  endings are however many 0x0A bytes happened to fall in it, and `edit safety:
  byte-exact` reads as a go-ahead where line and match semantics do not apply.
  The output is now `path`, `bytes` and a `not text:` line naming the reason.
  `intact info FILE --force` prints the full report anyway, `--force` meaning
  here what it means everywhere else, so `info` now accepts it.
- Under `--json`, `info` on such a file returns `ok`, `command`, `path`,
  `bytes`, `looks_binary` and a `binary` object (`reason`, `offset`, `detail`),
  and omits every decoded field. `looks_binary` is always present, so it or
  `binary` can be tested before reading the rest. `--force` returns the full
  object as before. **This changes the `--json` shape for non-text files:** a
  consumer that read `encoding` or `eol` unconditionally must now check first.
- The mojibake warning is no longer reported for a non-text file at all, on any
  command. The mojibake shape is two ordinary bytes in sequence, so blobs turn
  it up constantly by chance — `/bin/ls` yielded 93 such sequences, `libc.so.6`
  1581 — and its advice, to report the damage rather than hand-fix it, is not
  advice about an ELF binary. This covers a `--force`d write as well as `info`.

### Added

- BOM-less UTF-16 is recognised as its own case rather than reported as
  binary. Nothing declares the encoding of such a file and detection cannot
  guess it, so it used to be a dead end; the refusal now names the flag that
  reads it (`pass --encoding utf-16le ...`).

## [0.3.0] - 2026-08-15

### Added

- Mojibake-shaped text is reported rather than silently edited around. `info`
  prints a `warning:` line for it and carries a `mojibake` object (`count`,
  `line`, `sample`) under `--json`; every command that writes prints the same
  warning on stderr before its result, and carries a `warnings` array under
  `--json`. It is an advisory, not a guard — the edit goes through, and
  `--quiet` does not silence it. When the encoding was inferred rather than
  declared the warning says so, because a wrong reading and real damage look
  alike. `intact guide encoding` explains what the shape can and cannot catch.

### Changed

- `similar` 2.7 to 3.1, and `chardetng` 0.1.17 to 1.0.
- The minimum supported Rust version is 1.85, raised from 1.74 by `similar` 3,
  which is a Rust 2024 edition crate.
- `intact` is built on the Rust 2024 edition, up from 2021.
- `--escapes` documents that it applies to text read with `--text-file`,
  `--find-file` and `--with-file` as well as to the inline arguments, which it
  always did.

### Fixed

- `intact append --help` and `intact write --help` still showed `--text-stdin`,
  removed in 0.2.0; the examples now use `--text-file -`.
- `intact write --help` listed `--strict-eol`, which `write` is exempt from —
  it replaces the whole content, so there are no existing line endings to
  enforce. `create` already omitted it.

## [0.2.0] - 2026-08-14

Making an edit visible to the person who approved it, and cutting the API down
to what an agent actually needs: one way to do each thing, and one command that
can make a whole changeset.

### Added

- `batch` operations take a `file` of their own, so one invocation can edit
  several files. The `FILE` argument is the default for operations that omit it,
  and may be left out when they all name one. Each file is decoded, checked and
  written back in its own encoding, so a script spanning a UTF-8 file and a
  windows-1252 one is fine. This is the only multi-file mode; every other
  command still takes exactly one file.
- Nothing is written until every operation in a script has succeeded, across
  every file it touches — a failure on the last operation leaves the first
  file untouched too.
- Any path argument that reads text accepts `-` for standard input, which is
  what `batch --script -` always did.
- `intact guide encoding` ends with the full list of encoding labels this build
  accepts.
- `--show-diff` applies an edit **and** prints a unified diff of what it
  changed, so one invocation both makes and reports the change. Preferable to a
  `--dry-run` followed by the real command, which is two approvals for one
  change and can drift between them.
- `--diff-context N` sets the unchanged lines shown either side of a change
  (default 3).
- Every `--json` edit result carries `edits` and `edit_count`: the spans that
  were actually replaced, each with `line`, `column`, `end_line`, `end_column`,
  `offset`, `end_offset`, `before` and `after`. This is what `intact` did,
  rather than what comparing two versions of the file suggests it did.
- `--json` adds `eol_before` and `eol_after` when line-ending styles change.
- Line-ending changes are reported above the diff whenever the styles in use
  change, e.g. `# line endings: lf=3 crlf=0 cr=0 -> lf=3 crlf=1 cr=0`. Terminators
  are still not compared line by line, since converting a file from LF to CRLF
  would otherwise report every line as changed.
- Diffs mark a missing trailing newline with `\ No newline at end of file`.
- `intact instructions` tells the other project's agent to pass `--show-diff`
  on edits, and to write global flags before the subcommand so that a preview
  can be pre-approved by a prefix rule.

### Removed

- **All environment variables**: `INTACT_ENCODING`, `INTACT_NO_GUESS`,
  `INTACT_EOL`, `INTACT_STRICT_EOL` and `INTACT_SHOW_DIFF`. `intact` is normally
  driven one command per shell — which is how an agent runs it — so an `export`
  in one invocation is gone by the next, and a mandate that holds only sometimes
  is worse than one that never holds. The corresponding flags are unchanged, and
  `intact instructions` now generates flag-based rules. `detected_by` no longer
  reports `environment`.
- `--text-stdin` and `--with-stdin`, in favour of `--text-file -` and
  `--with-file -`.
- `create --overwrite`, which was exactly `write`. `create` on an existing file
  exits 7 and points at `write`.
- `intact encodings`, folded into `intact guide encoding`.
- The `5..9` line-range spelling; `:` is the only separator.
- The visible aliases `set-lines` and `claude-md`. Both still resolve, but are
  no longer advertised as second names for one command.

### Changed

- `COMMAND --help` lists only the global options that command actually reads.
  `intact info --help` used to advertise `--backup`, `--dry-run`, `--unmappable`
  and eight more that `info` ignores; it now shows `--json` and `--encoding`.
  `create` drops `--backup` (the file cannot already exist), `convert` drops
  `--eol`/`--strict-eol` (it has `--newlines`), `delete` and `batch` drop
  `--escapes`. Nothing about parsing changed: every global is still accepted by
  every command, in either position, so passing `--encoding LABEL --no-guess`
  uniformly still works.
- An option that cannot do anything is refused (exit 2) rather than accepted
  and ignored. `--no-expand` without `--regex` did nothing; `--max 0` reported
  "no match" on a file full of matches; `--expect 0` could only ever exit 3 or
  4; `--occurrence 0` names no occurrence; `convert --bom add` to an encoding
  with no byte-order mark wrote no mark and said nothing. `batch` scripts get
  the same treatment for the two its JSON can express.
- `insert` states in its usage line that `--line` or `--after` is required, and
  every command taking text that `--text` or `--text-file` is, instead of
  opening the file and only then reporting it.
- `replace --find-file - --with-file -` is refused: there is one standard
  input, so the second read returned an empty string and the replacement
  silently became a deletion.
- `batch` reports per file: one summary line each, and under `--json` a `files`
  array with one object per file — always an array, whether the script touched
  one file or twenty. It replaces the previous single-file fields and `steps`.
- `--escapes` documents that it applies to `--with` as well as `--text` and
  `--find`, which it always did.
- Diffs are unified diffs: `---`/`+++` headers, context lines, and one hunk per
  change. `git apply -p0` accepts the output. Previously a diff was a single
  block spanning everything between the first and last change, capped at 40
  lines per side — on a 200-line file with two changes 190 lines apart, that was
  84 lines of output with the second change truncated out of sight.
- `--quiet` suppresses only the summary line, not a diff that was explicitly
  asked for. `intact --dry-run --quiet replace ... > change.patch` now writes
  the patch and nothing else.
- The `diff` field in `--json` output is never truncated; human output is
  capped at 400 rows, and says how many hunks it left out.
- Text-to-text alignment (used by `batch` and `convert`) is handled by the
  `similar` crate, the one new dependency.

### Fixed

- A dry run of `convert --newlines` reported `would change` and then printed no
  diff at all, because line terminators were stripped before comparison.
- Adding or removing a file's final newline was invisible in a preview.
- A change with no textual difference — re-encoding a file, adding a BOM — now
  says so instead of printing nothing under a `would change` summary.

## [0.1.0] - 2026-08-14

Initial release.

[Unreleased]: https://github.com/jonasdeyson/intact/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/jonasdeyson/intact/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/jonasdeyson/intact/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/jonasdeyson/intact/releases/tag/v0.1.0
