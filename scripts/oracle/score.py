"""Score one Scippy run against the stored scip-python index. It runs no indexer.

usage: python3 -I score.py CORPUS RUNDIR [CMPNAME] [--reuse-compare] [--label RUN] [--csv FILE]

RUNDIR holds the Scippy index (`scippy.scip` or `index.scip`) and its trace (`scippy.trace` or
`trace.tsv`). The scip-python index is RUNDIR/scip-python.scip, or ORACLE_DIR/ref/CORPUS.scip.
The results go to RUNDIR/CMPNAME (default `cmp`): summary.json, disagreements.jsonl,
buckets.json, buckets.txt, metrics.txt, metrics.json.
`--reuse-compare` keeps an existing compare.py output. `--label RUN` also appends the per-class
counts of the run to the CSV (see track.py).
"""
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import oraclelib  # noqa: E402


def run(args, out=None):
    cmd = [sys.executable, "-I"] + args
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE="1")
    if out is None:
        subprocess.run(cmd, check=True, env=env)
    else:
        with open(out, "w") as f:
            subprocess.run(cmd, check=True, env=env, stdout=f)


def score(corpus, rundir, cmpname="cmp", reuse=False):
    root = oraclelib.corpus_root(corpus)
    if not os.path.isdir(root):
        raise SystemExit("no corpus root: %s (see README.md, fetch_corpora.sh)" % root)
    scippy = oraclelib.scippy_index(rundir)
    trace = oraclelib.scippy_trace(rundir)
    ref = oraclelib.reference_index(corpus, rundir)
    out = os.path.join(rundir, cmpname)
    if not (reuse and os.path.exists(os.path.join(out, "disagreements.jsonl")) and os.path.exists(os.path.join(out, "summary.json"))):
        os.makedirs(out, exist_ok=True)
        run([os.path.join(HERE, "compare.py"), root, scippy, ref, out], out + ".stdout")
    run([os.path.join(HERE, "analyze.py"), root, out, scippy, trace, os.path.join(out, "buckets.json")], os.path.join(out, "buckets.txt"))
    run([os.path.join(HERE, "metrics.py"), out], os.path.join(out, "metrics.txt"))
    return out


def main():
    args = sys.argv[1:]
    reuse = "--reuse-compare" in args
    label = csv = None
    if "--label" in args:
        i = args.index("--label")
        label = args[i + 1]
        del args[i:i + 2]
    if "--csv" in args:
        i = args.index("--csv")
        csv = args[i + 1]
        del args[i:i + 2]
    args = [a for a in args if a != "--reuse-compare"]
    if len(args) not in (2, 3):
        raise SystemExit(__doc__)
    out = score(args[0], args[1], args[2] if len(args) == 3 else "cmp", reuse)
    print(open(os.path.join(out, "metrics.txt")).read(), end="")
    if label:
        import track
        track.add(csv or oraclelib.DEFAULT_CSV, label, {args[0]: out})


if __name__ == "__main__":
    main()
