#!/usr/bin/env bash
# Write the loci-core export of each Loci fixture vault to languages/loci/tests/oracle/<name>.jsonl.
#
# Usage: scripts/loci-oracle.sh [fixture names...]
#
# loci-core is the specification of the Loci pack. Its export is the oracle of the tests.
# Set LOCI_CORE to a checkout of loci-core. Set LOCI_PYTHON to a Python with its dependencies
# (default: `$LOCI_CORE/.venv/bin/python`).
# Use a scratch clone, so that a run never writes into a working repository.
# The script puts the loci cache in a temporary folder and writes no byte code.
# A fixture with a `vault` folder is a workspace. The script copies it to a temporary folder,
# writes the place of the roots over `@ROOTS@` in the `.toml` files, and makes a git repository
# of each root folder named `git_*`. The tests do the same.
set -euo pipefail

cd "$(dirname "$0")/.."
: "${LOCI_CORE:?set LOCI_CORE to a checkout of loci-core}"
fixtures=languages/loci/tests/fixtures
oracle=languages/loci/tests/oracle
cache=$(mktemp -d)
work=$(mktemp -d)
trap 'rm -rf "$cache" "$work"' EXIT
export PYTHONDONTWRITEBYTECODE=1 XDG_CACHE_HOME="$cache" PYTHONPATH="$LOCI_CORE/src:$LOCI_CORE"

mkdir -p "$oracle"
names=("$@")
[ ${#names[@]} -eq 0 ] && names=($(ls "$fixtures"))
for name in "${names[@]}"; do
  vault="$fixtures/$name"
  if [ -d "$vault/vault" ]; then
    cp -r "$vault" "$work/$name"
    find "$work/$name" -name '*.toml' -exec sed -i "s#@ROOTS@#$work/$name/roots#g" {} +
    for repo in "$work/$name"/roots/git_*; do
      [ -d "$repo" ] && git -C "$repo" init -q && git -C "$repo" add -A
    done
    vault="$work/$name/vault"
  fi
  "${LOCI_PYTHON:-$LOCI_CORE/.venv/bin/python}" -c '
import sys
from apps.cli.main import run
sys.argv = ["loci", "--vault", sys.argv[1], "--json", "graph/relations"]
sys.exit(run())' "$vault" | python3 -c '
import sys, json
d = json.load(sys.stdin)
assert d["ok"], d
for r in d["value"]["relations"]:
    print(json.dumps(r, ensure_ascii=False, sort_keys=True))
' > "$oracle/$name.jsonl"
  echo "$name: $(wc -l < "$oracle/$name.jsonl") records"
done
