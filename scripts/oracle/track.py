"""Per-class tracking of oracle runs in one CSV.

usage:
  python3 -I track.py add RUN CORPUS=CMPDIR [CORPUS=CMPDIR ...] [--csv FILE] [--date YYYY-MM-DD]
  python3 -I track.py delta RUN --baseline BASE [--corpus NAME] [--all] [--csv FILE]
  python3 -I track.py runs [--csv FILE]

`add` appends the rows of metrics.json (named numbers, legacy numbers, class counts) of each
comparison dir. It refuses a (RUN, CORPUS) pair that the CSV already has. `delta` prints, for each
corpus, the rows that differ between the named baseline run and RUN. The default CSV is
`classes.csv` next to this script (ORACLE_CSV overrides it).
Columns: run, date, corpus, kind (named, legacy, class), name, value.
"""
import csv
import datetime
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import oraclelib  # noqa: E402
from oraclelib import read_json  # noqa: E402

FIELDS = ["run", "date", "corpus", "kind", "name", "value"]


def read_rows(path):
    if not os.path.exists(path):
        return []
    with open(path, newline="") as f:
        return list(csv.DictReader(f))


def flatten(metrics):
    """Yield (kind, name, value) for a metrics.json dict. Class rows with a zero count are skipped."""
    for k, v in sorted(metrics["named"].items()):
        yield "named", k, v
    for k, v in sorted(metrics["legacy"].items()):
        if isinstance(v, dict):
            for k2, v2 in sorted(v.items()):
                yield "legacy", "%s.%s" % (k, k2), v2
        else:
            yield "legacy", k, v
    for k, v in sorted(metrics["classes"].items()):
        if v:
            yield "class", k, v


def add(path, run, cmpdirs, date=None):
    """Append the rows of each CORPUS -> CMPDIR to the CSV. Return the number of rows."""
    date = date or datetime.date.today().isoformat()
    have = {(r["run"], r["corpus"]) for r in read_rows(path)}
    new = []
    for corpus, d in cmpdirs.items():
        if (run, corpus) in have:
            raise SystemExit("%s already has run %r for %s. Use a new run name." % (path, run, corpus))
        metrics = read_json(os.path.join(d, "metrics.json"))
        for kind, name, value in flatten(metrics):
            new.append({"run": run, "date": date, "corpus": corpus, "kind": kind, "name": name, "value": value})
    exists = os.path.exists(path)
    with open(path, "a", newline="") as f:
        w = csv.DictWriter(f, FIELDS, lineterminator="\n")
        if not exists:
            w.writeheader()
        w.writerows(new)
    print("%s: appended %d rows for run %r" % (path, len(new), run))
    return len(new)


def run_values(rows, run):
    out = {}
    for r in rows:
        if r["run"] == run:
            out.setdefault(r["corpus"], {})[(r["kind"], r["name"])] = float(r["value"])
    return out


def delta(path, run, baseline, corpus=None, show_all=False, out=sys.stdout):
    rows = read_rows(path)
    names = {r["run"] for r in rows}
    for n in (run, baseline):
        if n not in names:
            raise SystemExit("%s has no run %r. Runs: %s" % (path, n, ", ".join(sorted(names)) or "none"))
    cur, base = run_values(rows, run), run_values(rows, baseline)
    for c in sorted(set(cur) & set(base)):
        if corpus and c != corpus:
            continue
        keys = set(cur[c]) | set(base[c])
        diffs = []
        for k in keys:
            a, b = base[c].get(k, 0.0), cur[c].get(k, 0.0)
            if a != b or show_all:
                diffs.append((k, a, b))
        out.write("== %s: %s against baseline %s: %d changed rows ==\n" % (c, run, baseline, sum(1 for _, a, b in diffs if a != b)))
        order = {"named": 0, "class": 1, "legacy": 2}
        for (kind, name), a, b in sorted(diffs, key=lambda x: (order[x[0][0]], -abs(x[2] - x[1]), x[0][1])):
            out.write("%-7s %-90s %12s -> %-12s %+g\n" % (kind, name[:90], "%g" % a, "%g" % b, round(b - a, 4)))
    for c in sorted(set(cur) ^ set(base)):
        out.write("corpus %s is only in run %s\n" % (c, run if c in cur else baseline))


def main():
    a = sys.argv[1:]
    csv_path = oraclelib.DEFAULT_CSV
    date = None
    corpus = None
    baseline = None
    show_all = False
    pos = []
    i = 0
    while i < len(a):
        if a[i] == "--csv":
            csv_path = a[i + 1]
            i += 2
        elif a[i] == "--date":
            date = a[i + 1]
            i += 2
        elif a[i] == "--baseline":
            baseline = a[i + 1]
            i += 2
        elif a[i] == "--corpus":
            corpus = a[i + 1]
            i += 2
        elif a[i] == "--all":
            show_all = True
            i += 1
        else:
            pos.append(a[i])
            i += 1
    if not pos:
        raise SystemExit(__doc__)
    cmd = pos[0]
    if cmd == "add" and len(pos) >= 3:
        pairs = dict(p.split("=", 1) for p in pos[2:])
        add(csv_path, pos[1], pairs, date)
    elif cmd == "delta" and len(pos) == 2 and baseline:
        delta(csv_path, pos[1], baseline, corpus, show_all)
    elif cmd == "runs":
        seen = {}
        for r in read_rows(csv_path):
            seen.setdefault(r["run"], (r["date"], set()))[1].add(r["corpus"])
        for run, (d, cs) in seen.items():
            print("%s\t%s\t%s" % (run, d, ",".join(sorted(cs))))
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
