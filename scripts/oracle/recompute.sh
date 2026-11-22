#!/usr/bin/env bash
# usage: recompute.sh CORPUS OUTDIR [CMPNAME]
# Compare, bucket, and print the named metrics from the indexes in OUTDIR/CORPUS. It does not index.
# The results go to OUTDIR/CORPUS/CMPNAME (default cmp). The scip-python index is
# OUTDIR/CORPUS/scip-python.scip, or ORACLE_DIR/ref/CORPUS.scip when the run has none.
set -eu
. "$(dirname "$0")/env.sh"
exec python3 -I "$HERE/score.py" "$1" "$2/$1" "${3:-cmp}"
