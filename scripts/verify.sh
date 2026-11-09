#!/usr/bin/env bash
# Verify the workspace. Run it inside the devenv shell:
#   devenv shell -- scripts/verify.sh
#
# The tests call the official `scip` CLI (lint, snapshot, test).
# The CLI must be on PATH, or in the directory that SCIP_BIN_DIR names.
# Install it from https://github.com/scip-code/scip/releases.
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ -n "${SCIP_BIN_DIR:-}" ]]; then
  PATH="$SCIP_BIN_DIR:$PATH"
fi
if ! command -v scip >/dev/null; then
  echo "verify: the scip CLI is not on PATH. Set SCIP_BIN_DIR or install it." >&2
  exit 1
fi
echo "verify: $(scip --version)"

# Make the tests fail, not skip, when the scip CLI is missing.
export SCIPPY_REQUIRE_SCIP=1

cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
echo "verify: ok"
