#!/usr/bin/env bash
# Regenerates MANUAL.md from `intact guide`, the manual built into the binary
# (src/manual/). Run this after changing anything under src/manual/ and
# commit the result; CI fails the build if the two drift apart.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --quiet

{
  cat <<'HEADER'
# intact — full manual

This file is generated from `intact guide` — the same manual built into the
binary, so it is never stale. Do not edit it by hand: run
`scripts/gen-manual.sh` after changing anything under `src/manual/`, then
commit the result.

For one topic at a time: `intact guide TOPIC` (list them with
`intact guide --list`, or see the table in [README.md](README.md#commands)).

```text
HEADER
  ./target/debug/intact guide
  echo '```'
} > MANUAL.md
