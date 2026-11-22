#!/usr/bin/env bash
# usage: run_gate.sh OUTDIR [CORPUS ...]
# Index each corpus with SCIPPY_BIN (graph cache off, with --trace) and run the oracle gate on the result.
# Default corpora: rich scrapy mypy. A mypy run takes minutes, so this script is not part of `pm verify`.
# Corpora must exist in ORACLE_DIR/corpora (fetch_corpora.sh), with a baseline in ORACLE_DIR/baseline.
# Exit code: 0 the gate passes, 1 a check fails, other: an error. See README.md.
set -u
. "$(dirname "$0")/env.sh"
: "${SCIPPY_BIN:?set SCIPPY_BIN to the Scippy binary}"
OUT=$1; shift
[ $# -gt 0 ] || set -- rich scrapy mypy
args=()
for c in "$@"; do
  mkdir -p "$OUT/$c"
  SCIPPY_GRAPH_CACHE=off "$SCIPPY_BIN" index "$ORACLE_DIR/corpora/$c/root" --output "$OUT/$c/scippy.scip" \
    --trace "$OUT/$c/scippy.trace" --package-name "$c" > "$OUT/$c/scippy.stdout" 2> "$OUT/$c/scippy.stderr"
  rc=$?
  # Exit code 3 means incomplete input (bounded references). The index is complete.
  [ $rc -eq 0 ] || [ $rc -eq 3 ] || { echo "$c: scippy exit $rc. See $OUT/$c/scippy.stderr" >&2; exit 2; }
  args+=("$c=$OUT/$c")
done
exec python3 -I "$HERE/gate.py" "${args[@]}"
