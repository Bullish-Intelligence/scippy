#!/usr/bin/env bash
# usage: run_all.sh OUTDIR
# Run the full oracle on mypy and rich, one index at a time. SCIPPY_BIN names the Scippy binary.
# Setup in ORACLE_DIR (default ~/.local/state/scippy/oracle):
#   scip-python:  npm --prefix $ORACLE_DIR/scip-python install @sourcegraph/scip-python   (0.6.8)
#   corpora:      mypy = mypy 1.19.1 sdist, .py files only, in corpora/mypy/root;
#                 rich = src/rich of https://github.com/Textualize/rich, in corpora/rich/root/rich
#   env-empty.json: an empty scip-python environment, `[]`.
set -eu
H=$(dirname "$0")
OUT=$1
for c in mypy rich; do
  for t in scip-python scippy; do "$H/run_tools.sh" $c $c $t "$OUT"; done
  "$H/run_trace.sh" $c $c "$OUT"
  "$H/recompute.sh" $c "$OUT"
done
