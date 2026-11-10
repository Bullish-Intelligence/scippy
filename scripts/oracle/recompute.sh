#!/usr/bin/env bash
# usage: recompute.sh CORPUS OUTDIR [CMPNAME]
# Compare, bucket, and print metrics from the indexes in OUTDIR/CORPUS. It does not index.
# The results go to OUTDIR/CORPUS/CMPNAME (default cmp).
set -eu
. "$(dirname "$0")/env.sh"
c=$1; D=$2/$c; N=${3:-cmp}
python3 -I "$HERE/compare.py" "$ORACLE_DIR/corpora/$c/root" "$D/scippy.scip" "$D/scip-python.scip" "$D/$N" > "$D/$N.stdout"
python3 -I "$HERE/analyze.py" "$ORACLE_DIR/corpora/$c/root" "$D/$N" "$D/scippy.scip" "$D/scippy.trace" "$D/$N/buckets.json" > "$D/$N/buckets.txt"
python3 -I "$HERE/metrics.py" "$D/$N" | tee "$D/$N/metrics.txt"
