"""Regression gate of the accuracy oracle. It runs no indexer. It is not part of `pm verify`.

usage: python3 -I gate.py CORPUS=RUNDIR [CORPUS=RUNDIR ...] [--baseline-dir DIR] [--allow FILE]
                          [--max-strict-drop PP] [--zero-wrong CORPUS,...] [--limit N]

RUNDIR holds a Scippy index and its trace (`scippy index --trace`) of the corpus. The gate scores
it against the stored scip-python index, then compares it with the baseline run of the same
corpus (default ORACLE_DIR/baseline/CORPUS, which holds a cmp/ dir from score.py).
A corpus fails when one of these holds:
  1. real wrong > 0 on a corpus of `--zero-wrong` (default mypy,rich);
  2. strict recall or strict precision falls by more than `--max-strict-drop` pp (default 0.05);
  3. a location changes from resolved to unresolved, or to a different target, and it is not in
     the allowlist (default gate-allow.txt next to this script);
  4. an occurrence of the candidate has more than one symbol.
Exit code 0: every corpus passes. 1: a check failed. 2: usage or input error.
Allowlist lines: `CORPUS<TAB>doc:LINE:COL<TAB>reason`. LINE is 1-based and COL is a 0-based code
point offset, the format that this gate prints. `#` starts a comment line.
"""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import difftool  # noqa: E402
import oraclelib  # noqa: E402
from oraclelib import read_json  # noqa: E402
import score  # noqa: E402


def read_allow(path):
    """Return {(corpus, location): reason}."""
    out = {}
    if not os.path.exists(path):
        return out
    for n, line in enumerate(open(path), 1):
        line = line.rstrip("\n")
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) < 3 or not parts[2].strip():
            raise SystemExit("%s:%d: expected CORPUS<TAB>doc:LINE:COL<TAB>reason" % (path, n))
        out[(parts[0], parts[1])] = parts[2]
    return out


def baseline_metrics(corpus, bdir):
    p = os.path.join(bdir, "cmp", "metrics.json")
    if not os.path.exists(p):
        score.score(corpus, bdir, "cmp", reuse=True)
    return read_json(p)


def check(corpus, rundir, bdir, allow, max_drop, zero_wrong, limit):
    """Return (list of failure texts, list of notes) for one corpus."""
    fails, notes = [], []
    root = oraclelib.corpus_root(corpus)
    cmpdir = score.score(corpus, rundir, "gate-cmp")
    cur = read_json(os.path.join(cmpdir, "metrics.json"))["named"]
    base = baseline_metrics(corpus, bdir)["named"]

    # 1. real wrong
    if corpus in zero_wrong and cur["real_wrong"] > 0:
        fails.append("real wrong is %d on %s (must be 0). See %s/buckets.txt" % (cur["real_wrong"], corpus, cmpdir))
    notes.append("real wrong %d (baseline %d)" % (cur["real_wrong"], base["real_wrong"]))

    # 2. strict match
    for name in ("strict_recall_pct", "strict_precision_pct"):
        drop = base[name] - cur[name]
        notes.append("%s %.4f (baseline %.4f, change %+.4f pp)" % (name, cur[name], base[name], 0.0 - drop))
        if drop > max_drop + 1e-9:
            fails.append("%s fell by %.4f pp (limit %.4f): %.4f -> %.4f" % (name, drop, max_drop, base[name], cur[name]))

    # 3. locations that changed, 4. several symbols at one occurrence
    bt, _, _, _ = difftool.load_positions(oraclelib.scippy_index(bdir), root)
    ct, multi, _, _ = difftool.load_positions(oraclelib.scippy_index(rundir), root)
    d = difftool.diff(bt, ct)
    changed = [(p, "resolved -> unresolved") for p in d["lost"]] + [(p, "target changed") for p in d["changed"]]
    bad = [(difftool.fmt(p), why) for p, why in changed if (corpus, difftool.fmt(p)) not in allow]
    allowed = [x for x in changed if (corpus, difftool.fmt(x[0])) in allow]
    notes.append("locations: lost %d, changed %d, gained %d; allowed %d" % (len(d["lost"]), len(d["changed"]), len(d["gained"]), len(allowed)))
    if bad:
        fails.append("%d location(s) changed from resolved and are not in the allowlist" % len(bad))
        for loc, why in bad[:limit]:
            fails.append("    %s  %s" % (loc, why))
        if len(bad) > limit:
            fails.append("    ... %d more (raise --limit)" % (len(bad) - limit))
    changed_locs = {difftool.fmt(p) for p, _ in changed}
    stale = sorted(loc for (c, loc) in allow if c == corpus and loc not in changed_locs)
    if stale:
        notes.append("%d allowlist entr%s did not change (stale): %s" % (len(stale), "y" if len(stale) == 1 else "ies", ", ".join(stale[:5])))
    if multi:
        fails.append("%d occurrence(s) have more than one symbol" % len(multi))
        for doc, rng, syms in multi[:limit]:
            fails.append("    %s:%d:%d  %s" % (doc, rng[0] + 1, rng[1], syms))
    notes.append("occurrences with more than one symbol: %d" % len(multi))
    return fails, notes


def run(argv):
    """Run the gate on `CORPUS=RUNDIR` arguments and options. Return the exit code."""
    a = list(argv)
    opts = {"--baseline-dir": os.path.join(oraclelib.ORACLE_DIR, "baseline"), "--allow": oraclelib.DEFAULT_ALLOW,
            "--max-strict-drop": "0.05", "--zero-wrong": "mypy,rich", "--limit": "30"}
    for flag in list(opts):
        if flag in a:
            i = a.index(flag)
            opts[flag] = a[i + 1]
            del a[i:i + 2]
    pairs = []
    for x in a:
        if "=" not in x:
            sys.stderr.write(__doc__)
            return 2
        pairs.append(tuple(x.split("=", 1)))
    if not pairs:
        sys.stderr.write(__doc__)
        return 2
    allow = read_allow(opts["--allow"])
    zero = set(opts["--zero-wrong"].split(",")) - {""}
    status = 0
    for corpus, rundir in pairs:
        bdir = os.path.join(opts["--baseline-dir"], corpus)
        fails, notes = check(corpus, rundir, bdir, allow, float(opts["--max-strict-drop"]), zero, int(opts["--limit"]))
        print("== %s: %s ==" % (corpus, "FAIL" if fails else "pass"))
        for n in notes:
            print("  " + n)
        for f in fails:
            print("  FAIL " + f if not f.startswith("    ") else f)
        if fails:
            status = 1
    print("gate: %s" % ("FAIL" if status else "pass"))
    return status


if __name__ == "__main__":
    sys.exit(run(sys.argv[1:]))
