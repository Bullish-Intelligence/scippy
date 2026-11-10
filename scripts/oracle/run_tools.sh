#!/usr/bin/env bash
# usage: run_tools.sh CORPUS PROJECT_NAME TOOL OUTDIR
# Index ORACLE_DIR/corpora/CORPUS/root with TOOL (scippy or scip-python) into OUTDIR/CORPUS.
# SCIPPY_BIN names the Scippy binary. Each run has a 24 GB cap, a 30 min timeout, and GNU time -v.
set -u
. "$(dirname "$0")/env.sh"
C=$1; N=$2; T=$3; OUT=$4
ROOT=$ORACLE_DIR/corpora/$C/root
mkdir -p "$OUT/$C" "$OUT/logs" "$OUT/work"
cd "$OUT/work"
if [ "$T" = scippy ]; then
  CMD=("$SCIPPY_BIN" index "$ROOT" --output "$OUT/$C/scippy.scip" --package-name "$N")
else
  CMD=("$ORACLE_DIR/scip-python/node_modules/.bin/scip-python" index --cwd "$ROOT" --project-name "$N" --project-version 0.0.0 --environment "$ORACLE_DIR/env-empty.json" --output "$OUT/$C/scip-python.scip" --quiet)
fi
systemd-run --user --scope -p MemoryMax=24G -- timeout 1800 /run/current-system/sw/bin/time -v "${CMD[@]}" > "$OUT/logs/$C-$T.stdout" 2> "$OUT/logs/$C-$T.time"
echo "exit $?"
grep -E "Elapsed|Maximum resident|Exit status" "$OUT/logs/$C-$T.time"
