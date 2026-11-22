"""Shared paths of the oracle scripts. It reads no index and it runs no tool."""
import json
import os

# ORACLE_DIR holds corpora/NAME/root, ref/NAME.scip, and baseline/NAME (see README.md).
ORACLE_DIR = os.environ.get("ORACLE_DIR", os.path.expanduser("~/.local/state/scippy/oracle"))
HERE = os.path.dirname(os.path.abspath(__file__))
CORPORA = ["mypy", "rich", "stdlib", "scrapy", "celery", "pydantic-ai"]
# The default file of the per-class counts. Override it with ORACLE_CSV or `--csv`.
DEFAULT_CSV = os.environ.get("ORACLE_CSV", os.path.join(HERE, "classes.csv"))
DEFAULT_ALLOW = os.path.join(HERE, "gate-allow.txt")


def corpus_root(corpus):
    return os.path.join(ORACLE_DIR, "corpora", corpus, "root")


def first_file(rundir, names, what):
    for n in names:
        p = os.path.join(rundir, n)
        if os.path.exists(p):
            return p
    raise SystemExit("%s: no %s (looked for %s)" % (rundir, what, ", ".join(names)))


def scippy_index(rundir):
    return first_file(rundir, ["scippy.scip", "index.scip"], "Scippy index")


def scippy_trace(rundir):
    return first_file(rundir, ["scippy.trace", "trace.tsv"], "Scippy trace (run `scippy index --trace FILE`)")


def reference_index(corpus, rundir=None):
    """The scip-python index of a corpus: the file of the run dir, else ORACLE_DIR/ref/CORPUS.scip."""
    if rundir and os.path.exists(os.path.join(rundir, "scip-python.scip")):
        return os.path.join(rundir, "scip-python.scip")
    p = os.path.join(ORACLE_DIR, "ref", corpus + ".scip")
    if not os.path.exists(p):
        raise SystemExit("no scip-python index for %s: %s" % (corpus, p))
    return p


def read_json(path):
    with open(path) as f:
        return json.load(f)
