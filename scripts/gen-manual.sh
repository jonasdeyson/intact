#!/usr/bin/env bash
# Regenerates MANUAL.md from the manual built into the binary (src/manual/).
# Run this after changing anything under src/manual/ and commit the result;
# CI fails the build if the two drift apart.
#
# The whole file comes out of `intact guide --markdown`, preamble included:
# anything this script added on top would be one more thing that could go stale.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --quiet
./target/debug/intact guide --markdown > MANUAL.md
