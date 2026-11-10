#!/usr/bin/env bash
# usage: run_trace.sh CORPUS PROJECT_NAME OUTDIR
# Run Scippy again with --trace. The index is the same; the trace feeds analyze.py.
set -u
. "$(dirname "$0")/env.sh"
C=$1; N=$2; OUT=$3
mkdir -p "$OUT/$C" "$OUT/logs" "$OUT/work"
cd "$OUT/work"
systemd-run --user --scope -p MemoryMax=24G -- timeout 1800 /run/current-system/sw/bin/time -v "$SCIPPY_BIN" index "$ORACLE_DIR/corpora/$C/root" --output "$OUT/$C/scippy-trace.scip" --trace "$OUT/$C/scippy.trace" --package-name "$N" > "$OUT/logs/$C-scippy-trace.stdout" 2> "$OUT/logs/$C-scippy-trace.time"
echo "exit $?"
grep -E "Elapsed|Maximum resident|Exit status" "$OUT/logs/$C-scippy-trace.time"
