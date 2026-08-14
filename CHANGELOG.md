# Changelog

All notable changes to this project are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Making an edit visible to the person who approved it. An agent's permission
prompt shows a command line, not a diff, and by the time `intact` prints
anything the edit is already approved — so the diff it prints afterwards has to
be worth reading.

### Added

- `--show-diff` applies an edit **and** prints a unified diff of what it
  changed, so one invocation both makes and reports the change. Preferable to a
  `--dry-run` followed by the real command, which is two approvals for one
  change and can drift between them.
- `INTACT_SHOW_DIFF=1` sets `--show-diff` for every invocation, for a project
  where every edit should show its work.
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

### Changed

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

[Unreleased]: https://github.com/jonasdeyson/intact/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/jonasdeyson/intact/releases/tag/v0.1.0
