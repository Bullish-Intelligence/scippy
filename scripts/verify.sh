#!/usr/bin/env bash
# Quick local check for the edit loop. Run it inside the devenv shell:
#   devenv shell -- scripts/verify.sh
#
# This script is not the gate. `testee verify` is the gate.
# Run `devenv shell -- testee verify` before you land.
# The quick profile runs check:lint, check:format, and test:quick (library tests only).
# It never reports GREEN. The Testee tasks in devenv.nix set up the scip CLI.
set -euo pipefail

cd "$(dirname "$0")/.."

testee verify --profile quick "$@"
