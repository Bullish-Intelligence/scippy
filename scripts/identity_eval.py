#!/usr/bin/env python3
"""Evaluate the entity rebinding of Scippy (METALAYER_IDENTITY.md section 9.C and 6 "Cost").

Run it through scripts/identity-eval.sh. Two subcommands exist.

  replay    Replay consecutive commit pairs of a COPIED git repository. Publish the parent, then
            the child, into one database. Compare each bind with the ground truth.
  overhead  Time the rebinding. Compare `--no-identity` runs with identity runs.

This tool is not part of the gate. It uses only the Python standard library.
"""

import argparse
import ast
import collections
import csv
import hashlib
import json
import os
import resource
import shutil
import sqlite3
import statistics
import subprocess
import sys
import tempfile

SEP = "\x1f"
TIERS = ["T1", "T1h", "T2f", "T2", "T3", "T4", "T5", "Param"]
# The size floor of the truth proxy, in Python `ast` nodes. The engine uses named tree-sitter nodes.
TRUTH_MIN_NODES = 10


def run(cmd, cwd=None, check=True, env=None, stdout=subprocess.PIPE):
    result = subprocess.run(cmd, cwd=cwd, env=env, stdout=stdout, stderr=subprocess.PIPE)
    if check and result.returncode:
        raise RuntimeError("%s failed: %s" % (cmd[:3], result.stderr.decode("utf-8", "replace")))
    return result


def git(repo, *args, check=True):
    return run(["git", "-C", repo, *args], check=check).stdout


# ---- Scippy runs ----


def index(binary, root, work, db=None, identity=None, no_identity=False, extra=()):
    """Run `scippy index` on ROOT. Return the CPU time (user and system) in seconds.

    CPU time stays stable under machine load. Wall time does not.
    """
    cmd = [binary, "index", root, "--output", os.path.join(work, "out.scip")]
    if db:
        cmd += ["--metalayer", db]
        if no_identity:
            cmd += ["--no-identity"]
        elif identity:
            cmd += ["--identity", identity]
    cmd += list(extra)
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    result = run(cmd, cwd=work, check=False)
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    elapsed = (after.ru_utime - before.ru_utime) + (after.ru_stime - before.ru_stime)
    if result.returncode:
        raise RuntimeError("scippy failed: " + result.stderr.decode("utf-8", "replace")[-500:])
    return elapsed


def copy_sqlite(src, dst):
    """Copy a SQLite database with the backup API. The WAL file stays consistent."""
    if os.path.exists(dst):
        os.remove(dst)
    source = sqlite3.connect(src)
    target = sqlite3.connect(dst)
    source.backup(target)
    target.close()
    source.close()


def snapshot(identity_db):
    """Read the identity store: entities with the current locator, and the outcome rows."""
    con = sqlite3.connect("file:%s?mode=ro" % identity_db, uri=True)
    ents = {}
    for row in con.execute(
        "SELECT e.id, e.kind, e.parent_id, l.key, l.path, l.name, l.container "
        "FROM entity e JOIN locator l ON l.entity_id = e.id AND l.seq = 0"
    ):
        ents[row[0]] = dict(
            id=row[0],
            kind=row[1],
            parent=row[2],
            key=row[3],
            path=row[4],
            name=row[5],
            cont=tuple(row[6].split(SEP)) if row[6] else (),
        )
    outcomes = {}
    for row in con.execute("SELECT entity_id, outcome, tier, candidates FROM outcome"):
        outcomes[row[0]] = dict(outcome=row[1], tier=row[2], candidates=row[3])
    con.close()
    return ents, outcomes


def tuple_of(ent):
    return (ent["path"], ent["cont"], ent["name"], ent["kind"])


# ---- Python `ast` proxy ----


def h(text):
    return hashlib.sha1(text.encode()).hexdigest()[:12]


def count_nodes(node):
    return sum(1 for _ in ast.walk(node))


def strip_doc(body):
    if (
        body
        and isinstance(body[0], ast.Expr)
        and isinstance(getattr(body[0], "value", None), ast.Constant)
        and isinstance(body[0].value.value, str)
    ):
        return body[1:]
    return body


def sig_digest(fn):
    a = fn.args
    parts = []
    for x in a.posonlyargs + a.args + a.kwonlyargs:
        parts.append(x.arg + ":" + (ast.dump(x.annotation) if x.annotation else ""))
    if a.vararg:
        parts.append("*" + a.vararg.arg)
    if a.kwarg:
        parts.append("**" + a.kwarg.arg)
    parts.append("nd=%d" % len(a.defaults))
    parts.append("->" + (ast.dump(fn.returns) if fn.returns else ""))
    return h("|".join(parts))


def extract(src):
    """Return {(container, name, kind): (sig, body, size)} for the global declarations."""
    tree = ast.parse(src)
    out = {}

    def add(cont, name, kind, sig, body, size):
        out.setdefault((cont, name, kind), (sig, body, size))

    def walk(stmts, cont, in_class):
        for s in stmts:
            if isinstance(s, (ast.FunctionDef, ast.AsyncFunctionDef)):
                body = strip_doc(s.body)
                add(
                    cont,
                    s.name,
                    "method" if in_class else "function",
                    sig_digest(s),
                    h("".join(ast.dump(x) for x in body) + "".join(ast.dump(d) for d in s.decorator_list)),
                    sum(count_nodes(x) for x in body),
                )
            elif isinstance(s, ast.ClassDef):
                body = strip_doc(s.body)
                add(cont, s.name, "class", "", h("".join(ast.dump(x) for x in body)), sum(count_nodes(x) for x in body))
                walk(s.body, cont + (s.name,), True)
            elif isinstance(s, (ast.If, ast.Try, ast.With)):
                for field in ("body", "orelse", "finalbody"):
                    walk(getattr(s, field, []), cont, in_class)
                for handler in getattr(s, "handlers", []):
                    walk(handler.body, cont, in_class)

    walk(tree.body, (), False)
    return out


class Proxy:
    """The `ast` facts of one revision, read lazily from git."""

    def __init__(self, repo, rev):
        self.repo, self.rev, self.cache = repo, rev, {}

    def decls(self, path):
        if path not in self.cache:
            result = run(["git", "-C", self.repo, "show", "%s:%s" % (self.rev, path)], check=False)
            try:
                self.cache[path] = extract(result.stdout.decode("utf-8", "replace"))
            except (SyntaxError, ValueError):
                self.cache[path] = {}
        return self.cache[path]

    def get(self, tup):
        path, cont, name, kind = tup
        return self.decls(path).get((cont, name, kind))

    def blob(self, path):
        return git(self.repo, "rev-parse", "%s:%s" % (self.rev, path), check=False).strip()


# ---- Ground truth ----


def file_changes(repo, parent, child):
    """Return (renamed {old: new}, deleted set, added set, modified set) of `git diff -M -C`."""
    renamed, deleted, added, modified = {}, set(), set(), set()
    text = git(repo, "diff", "-M", "-C", "--name-status", parent, child).decode("utf-8", "replace")
    for line in text.splitlines():
        parts = line.split("\t")
        status = parts[0][0]
        if status == "R":
            renamed[parts[1]] = parts[2]
        elif status == "C":
            added.add(parts[2])
        elif status == "D":
            deleted.add(parts[1])
        elif status == "A":
            added.add(parts[1])
        else:
            modified.add(parts[1])
    return renamed, deleted, added, modified


def truth_for(a_ents, b_decls, renamed, deleted, proxy_a, proxy_b):
    """Return {old id: (class, [tuples])}. Class: kept, moved, ambiguous, gone.

    `b_decls` is the set of declaration tuples of the child revision.
    """
    truth = {}
    counterpart = {}
    for ent in a_ents.values():
        path, cont, name, kind = tuple_of(ent)
        if path in deleted:
            continue
        mapped = (renamed.get(path, path), cont, name, kind)
        if mapped in b_decls:
            counterpart[ent["id"]] = mapped
    taken = set(counterpart.values())
    added = [t for t in b_decls if t not in taken]
    by_hash = collections.defaultdict(list)
    for t in added:
        info = proxy_b.get(t) if t[0].endswith(".py") else None
        if info and info[2] >= TRUTH_MIN_NODES and t[3] in ("function", "method", "class"):
            by_hash[(t[3], info[1])].append(t)
    by_name = collections.defaultdict(list)
    for t in added:
        by_name[(t[3], t[2])].append(t)
    for ent in a_ents.values():
        tup = tuple_of(ent)
        if ent["id"] in counterpart:
            truth[ent["id"]] = ("kept", [counterpart[ent["id"]]])
            continue
        info = proxy_a.get(tup) if tup[0].endswith(".py") else None
        cands = []
        if info and info[2] >= TRUTH_MIN_NODES and tup[3] in ("function", "method", "class"):
            cands = by_hash.get((tup[3], info[1]), [])
            if len(cands) > 1:
                same_name = [c for c in cands if c[2] == tup[2]]
                if len(same_name) == 1:
                    cands = same_name
        if not cands and info and tup[3] in ("function", "method"):
            named = [c for c in by_name.get((tup[3], tup[2]), []) if (proxy_b.get(c) or (None,))[0] == info[0]]
            cands = named if len(named) == 1 else []
        if len(cands) == 1:
            truth[ent["id"]] = ("moved", cands)
        elif len(cands) > 1:
            truth[ent["id"]] = ("ambiguous", cands)
        else:
            truth[ent["id"]] = ("gone", [])
    return truth


# ---- Replay ----


def has_moves(repo, rev):
    """True when the commit renames a Python file, or adds and deletes Python files."""
    text = git(repo, "diff", "-M", "-C", "--name-status", rev + "^", rev, "--", "*.py").decode("utf-8", "replace")
    status = {line.split("\t")[0][0] for line in text.splitlines() if line}
    return bool(status & {"R", "C"}) or {"A", "D"} <= status


def pick_pairs(repo, count, moves=0):
    """Pick COUNT commits. Up to MOVES of them are likely refactorings. The rest are spread evenly."""
    revs = git(repo, "log", "--format=%H", "--no-merges", "--", "*.py").decode().split()
    revs = [r for r in revs if git(repo, "rev-parse", "--verify", "-q", r + "^", check=False).strip()]
    chosen = []
    if moves:
        movers = [r for r in revs if has_moves(repo, r)]
        step = max(1, len(movers) // moves)
        chosen = movers[::step][:moves]
    rest = [r for r in revs if r not in chosen]
    room = count - len(chosen)
    if len(rest) > room:
        step = len(rest) / room
        rest = [rest[int(i * step)] for i in range(room)]
    revs = chosen + rest
    revs = sorted(set(revs), key=revs.index)[:count]
    revs.reverse()
    return revs


def export(repo, rev, dest):
    for name in os.listdir(dest):
        path = os.path.join(dest, name)
        shutil.rmtree(path) if os.path.isdir(path) and not os.path.islink(path) else os.remove(path)
    archive = subprocess.Popen(["git", "-C", repo, "archive", rev], stdout=subprocess.PIPE)
    subprocess.run(["tar", "-x", "-C", dest], stdin=archive.stdout, check=True)
    archive.wait()


def is_python(ent):
    return ent["path"].endswith(".py")


def evaluate_pair(repo, commit, binary, work, extra, time_reps):
    parent = commit + "^"
    root = os.path.join(work, "root")
    db = os.path.join(work, "layer.sqlite")
    identity = os.path.join(work, "layer.identity.db")
    for f in (db, db + "-wal", db + "-shm", identity, identity + "-wal", identity + "-shm"):
        if os.path.exists(f):
            os.remove(f)
    export(repo, parent, root)
    index(binary, root, work, db, identity, extra=extra)
    a_ents, _ = snapshot(identity)
    timing = None
    if time_reps:
        a_db, a_id = db + ".a", identity + ".a"
        copy_sqlite(db, a_db)
        copy_sqlite(identity, a_id)
    export(repo, commit, root)
    index(binary, root, work, db, identity, extra=extra)
    b_ents, outcomes = snapshot(identity)
    if time_reps:
        timing = time_pair(binary, root, work, a_db, a_id, time_reps, extra)

    renamed, deleted, added, modified = file_changes(repo, parent, commit)
    # The declarations of the child: every entity except the stale old ones.
    stale = {i for i, o in outcomes.items() if o["outcome"] in ("ambiguous", "orphaned")}
    b_decls = {tuple_of(e) for i, e in b_ents.items() if i not in stale and e["kind"] != "parameter" and is_python(e)}
    a_code = {i: e for i, e in a_ents.items() if e["kind"] != "parameter" and is_python(e)}
    proxy_a, proxy_b = Proxy(repo, parent), Proxy(repo, commit)
    truth = truth_for(a_code, b_decls, renamed, deleted, proxy_a, proxy_b)

    counts = collections.Counter()
    detail = []
    bound_new = set()
    for oid, ent in a_code.items():
        cls, tuples = truth[oid]
        out = outcomes.get(oid)
        kind = ent["kind"]
        counts["old"] += 1
        counts["old:" + cls] += 1
        if out and out["outcome"] in ("ambiguous", "orphaned"):
            state = out["outcome"]
            counts[state] += 1
            counts["%s:truth-%s" % (state, cls)] += 1
            if state == "orphaned" and cls in ("kept", "moved"):
                detail.append(row(commit, "false-orphan", "", ent, tuples, None))
            continue
        tier = out["tier"] if out and out["outcome"] == "moved" else "T1"
        bound = tuple_of(b_ents[oid])
        bound_new.add(oid)
        ok = cls in ("kept", "moved", "ambiguous") and bound in tuples
        counts["bind:" + tier] += 1
        counts["bind:%s:%s" % (tier, cls)] += 1
        counts["bind:%s:kind:%s" % (tier, kind)] += 1
        if ok:
            counts["correct:" + tier] += 1
            counts["correct:%s:%s" % (tier, cls)] += 1
        else:
            detail.append(row(commit, "wrong-bind", tier, ent, tuples, bound))
    counts["minted"] = sum(1 for i, e in b_ents.items() if i not in a_ents and e["kind"] != "parameter" and is_python(e))
    counts["params:old"] = sum(1 for e in a_ents.values() if e["kind"] == "parameter" and is_python(e))
    counts["params:orphaned"] = sum(
        1 for i, e in a_ents.items() if e["kind"] == "parameter" and is_python(e) and outcomes.get(i, {}).get("outcome") == "orphaned"
    )
    counts["params:minted"] = sum(1 for i, e in b_ents.items() if i not in a_ents and e["kind"] == "parameter" and is_python(e))
    counts["file:renamed"] = len(renamed)
    counts["file:added"] = len(added)
    counts["file:deleted"] = len(deleted)
    counts["entities:all"] = len(a_ents)
    return counts, detail, timing


def row(commit, label, tier, ent, truth_tuples, bound):
    fmt = lambda t: "" if t is None else "%s::%s.%s [%s]" % (t[0], ".".join(t[1]), t[2], t[3])
    return dict(
        commit=commit[:8],
        label=label,
        tier=tier,
        old=fmt(tuple_of(ent)),
        bound=fmt(bound),
        truth=";".join(fmt(t) for t in truth_tuples),
    )


def pct(num, den):
    return "n/a" if not den else "%.1f%% (%d/%d)" % (100.0 * num / den, num, den)


def report(total, pairs):
    lines = []
    lines.append("Pairs: %d. Old Python declarations: %d." % (pairs, total["old"]))
    lines.append("")
    lines.append("| Tier | Binds | Correct | Precision |")
    lines.append("|---|---|---|---|")
    for tier in TIERS:
        if total["bind:" + tier]:
            lines.append("| %s | %d | %d | %s |" % (tier, total["bind:" + tier], total["correct:" + tier], pct(total["correct:" + tier], total["bind:" + tier])))
    lines.append("")
    lines.append("| Truth class | Old | Bound correctly | Recall | Bound by T1 | Bound by another tier | Ambiguous | Orphaned |")
    lines.append("|---|---|---|---|---|---|---|---|")
    for cls in ("kept", "moved", "ambiguous", "gone"):
        n = total["old:" + cls]
        good = sum(total["correct:%s:%s" % (t, cls)] for t in TIERS)
        by_t1 = total["bind:T1:%s" % cls]
        by_other = sum(total["bind:%s:%s" % (t, cls)] for t in TIERS if t != "T1")
        lines.append(
            "| %s | %d | %d | %s | %d | %d | %d | %d |"
            % (cls, n, good, pct(good, n) if cls != "gone" else "n/a", by_t1, by_other, total["ambiguous:truth-" + cls], total["orphaned:truth-" + cls])
        )
    lines.append("")
    lines.append("Orphan rate: %s. Ambiguity rate: %s. True orphans (truth gone): %s. False orphans: %d."
                 % (pct(total["orphaned"], total["old"]), pct(total["ambiguous"], total["old"]),
                    pct(total["orphaned:truth-gone"], total["old:gone"]), total["orphaned:truth-kept"] + total["orphaned:truth-moved"]))
    lines.append("Minted: %d. Parameters: %d old, %d orphaned, %d minted." % (total["minted"], total["params:old"], total["params:orphaned"], total["params:minted"]))
    return "\n".join(lines)


def cmd_replay(args):
    repo = os.path.abspath(args.repo)
    work = os.path.abspath(args.work)
    os.makedirs(os.path.join(work, "root"), exist_ok=True)
    binary = os.path.abspath(args.scippy)
    extra = args.extra
    commits = pick_pairs(repo, args.pairs, args.moves)
    total = collections.Counter()
    details, timings = [], []
    per_pair = []
    for n, commit in enumerate(commits):
        reps = args.time_reps if n < args.time_pairs else 0
        try:
            counts, detail, timing = evaluate_pair(repo, commit, binary, work, extra, reps)
        except RuntimeError as err:
            print("pair %s skipped: %s" % (commit[:8], str(err)[:200]), file=sys.stderr)
            continue
        total.update(counts)
        details += detail
        per_pair.append(dict(commit=commit[:8], **counts))
        if timing:
            timings.append(timing)
        print("pair %d/%d %s done" % (n + 1, len(commits), commit[:8]), file=sys.stderr)
    out = os.path.join(work, os.path.basename(repo.rstrip("/")))
    keys = sorted({k for r in per_pair for k in r if k != "commit"})
    with open(out + "-pairs.csv", "w", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=["commit"] + keys)
        writer.writeheader()
        writer.writerows(per_pair)
    with open(out + "-detail.csv", "w", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=["commit", "label", "tier", "old", "bound", "truth"])
        writer.writeheader()
        writer.writerows(details)
    print(report(total, len(per_pair)))
    if timings:
        print()
        print(timing_report(timings))


# ---- Timing ----


def median(values):
    return statistics.median(values) if values else float("nan")


def time_pair(binary, root, work, a_db, a_id, reps, extra):
    """Time the publish of the child state on top of the saved parent state.

    r0: index, no metalayer. r1: index and publish with `--no-identity`.
    r2: index and publish with identity. n1, n2: the same publish with no change.
    """
    result = collections.defaultdict(list)
    db = os.path.join(work, "timing.sqlite")
    ident = os.path.join(work, "timing.identity.db")
    for _ in range(reps):
        result["r0"].append(index(binary, root, work, extra=extra))
        for tag, no_identity in (("r1", True), ("r2", False)):
            copy_sqlite(a_db, db)
            copy_sqlite(a_id, ident)
            result[tag].append(index(binary, root, work, db, ident, no_identity=no_identity, extra=extra))
            result["n" + tag[1]].append(index(binary, root, work, db, ident, no_identity=no_identity, extra=extra))
    return {k: median(v) for k, v in result.items()}


def timing_report(timings):
    med = {k: median([t[k] for t in timings]) for k in ("r0", "r1", "r2", "n1", "n2")}
    rebind = med["r2"] - med["r1"]
    publish = med["r1"] - med["r0"]
    noop = med["n2"] - med["n1"]
    lines = [
        "Timing over %d pairs (median CPU seconds, user and system): index only %.3f, publish without identity %.3f, publish with identity %.3f."
        % (len(timings), med["r0"], med["r1"], med["r2"]),
        "Rebinding overhead: %.3f s. Of the full run (no identity): %.1f%%. Of the publish step alone: %.1f%%."
        % (rebind, 100 * rebind / med["r1"], 100 * rebind / publish if publish > 0 else float("nan")),
        "No-op publish: with identity %.3f s, without %.3f s, extra %.3f s (%.1f%% of the no-op run)."
        % (med["n2"], med["n1"], noop, 100 * noop / med["n1"]),
    ]
    return "\n".join(lines)


# ---- Overhead on a synthetic corpus ----


def write_module(path, index_no, funcs, variant):
    lines = ["VALUE_%d = %d\n\n" % (index_no, index_no)]
    for f in range(funcs):
        extra = " + %d" % variant if variant and f == 0 else ""
        lines.append(
            "def func_%d_%d(alpha, beta):\n"
            "    total = alpha + beta * %d%s\n"
            "    items = [total, alpha, beta]\n"
            "    for item in items:\n"
            "        total = total + item * %d\n"
            "    return total\n\n\n" % (index_no, f, f + 2, extra, f + index_no)
        )
    lines.append("class Box_%d:\n" % index_no)
    for m in range(3):
        lines.append(
            "    def method_%d(self, value):\n"
            "        self.value = value + %d\n"
            "        items = [value, self.value]\n"
            "        return sum(items) * %d\n\n" % (m, m, index_no + m)
        )
    with open(path, "w") as f:
        f.write("".join(lines))


def make_corpus(root, files, variant):
    os.makedirs(os.path.join(root, "pkg"), exist_ok=True)
    with open(os.path.join(root, "pyproject.toml"), "w") as f:
        f.write('[project]\nname = "synthetic"\nversion = "1.0"\n')
    open(os.path.join(root, "pkg", "__init__.py"), "w").close()
    for i in range(files):
        write_module(os.path.join(root, "pkg", "mod_%d.py" % i), i, 8, variant if i % 10 == 0 else 0)


def cmd_overhead(args):
    binary = os.path.abspath(args.scippy)
    work = tempfile.mkdtemp(prefix="identity-bench-", dir=args.work)
    try:
        root = os.path.join(work, "root")
        os.makedirs(root)
        if args.root:
            shutil.copytree(args.root, root, dirs_exist_ok=True)
        else:
            make_corpus(root, args.files, 0)
        db, ident = os.path.join(work, "layer.sqlite"), os.path.join(work, "layer.identity.db")
        index(binary, root, work, db, ident)
        a_db, a_id = db + ".a", ident + ".a"
        copy_sqlite(db, a_db)
        copy_sqlite(ident, a_id)
        if not args.root:
            make_corpus(root, args.files, 1)
        timing = time_pair(binary, root, work, a_db, a_id, args.reps, [])
        print(timing_report([timing]))
    finally:
        shutil.rmtree(work, ignore_errors=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    rp = sub.add_parser("replay", help="replay commit pairs of a copied repository")
    rp.add_argument("repo", help="a COPY of a git repository")
    rp.add_argument("--work", required=True, help="scratch folder")
    rp.add_argument("--scippy", required=True, help="the scippy binary")
    rp.add_argument("--pairs", type=int, default=20)
    rp.add_argument("--moves", type=int, default=0, help="include up to K commits that rename or move files")
    rp.add_argument("--time-pairs", type=int, default=0, help="time the first K pairs")
    rp.add_argument("--time-reps", type=int, default=1)
    rp.add_argument("--extra", nargs=argparse.REMAINDER, default=[], help="extra `scippy index` arguments")
    rp.set_defaults(func=cmd_replay)
    op = sub.add_parser("overhead", help="time the rebinding")
    op.add_argument("--scippy", required=True)
    op.add_argument("--work", default=None, help="scratch parent folder")
    op.add_argument("--root", default=None, help="a project to copy. The default is a synthetic corpus")
    op.add_argument("--files", type=int, default=60, help="files of the synthetic corpus")
    op.add_argument("--reps", type=int, default=3)
    op.set_defaults(func=cmd_overhead)
    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
