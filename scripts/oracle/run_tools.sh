#!/usr/bin/env bash
# usage: run_tools.sh CORPUS PROJECT_NAME TOOL OUTDIR
# Index ORACLE_DIR/corpora/CORPUS/root with TOOL (scippy or scip-python) into OUTDIR/CORPUS.
# SCIPPY_BIN names the Scippy binary. ORACLE_VENV=1 runs scip-python in the environment that
# mkenv.sh built (ORACLE_ENVS/CORPUS). The default is the empty environment. Each run has a 24 GB cap, a 30 min timeout, and GNU time -v.
set -u
. "$(dirname "$0")/env.sh"
C=$1; N=$2; T=$3; OUT=$4
ROOT=$ORACLE_DIR/corpora/$C/root
mkdir -p "$OUT/$C" "$OUT/logs" "$OUT/work"
cd "$OUT/work"
if [ "$T" = scippy ]; then
  CMD=("$SCIPPY_BIN" index "$ROOT" --output "$OUT/$C/scippy.scip" --package-name "$N")
else
  ENVJSON=$ORACLE_DIR/env-empty.json
  if [ "${ORACLE_VENV:-0}" = 1 ]; then
    ENVJSON=$ORACLE_ENVS/$C/env.json
    export PATH=$ORACLE_ENVS/$C/venv/bin:$PATH VIRTUAL_ENV=$ORACLE_ENVS/$C/venv
  fi
  CMD=("$ORACLE_DIR/scip-python/node_modules/.bin/scip-python" index --cwd "$ROOT" --project-name "$N" --project-version 0.0.0 --environment "$ENVJSON" --output "$OUT/$C/scip-python.scip" --quiet)
fi
systemd-run --user --scope -p MemoryMax=24G -- timeout 1800 /run/current-system/sw/bin/time -v "${CMD[@]}" > "$OUT/logs/$C-$T.stdout" 2> "$OUT/logs/$C-$T.time"
echo "exit $?"
grep -E "Elapsed|Maximum resident|Exit status" "$OUT/logs/$C-$T.time"
