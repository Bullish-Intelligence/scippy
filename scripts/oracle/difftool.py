"""Per-location diff of two Scippy indexes of one corpus. It runs no indexer.

usage: python3 -I difftool.py CORPUS BASE_RUNDIR CAND_RUNDIR [--out PREFIX] [--limit N]

A location is a reference occurrence: (document, line, column). The target of a location is the
set of definition sites of its symbols in the index of its own run, or the symbol text when the
symbol has no definition in the project. The tool lists these changes from BASE to CAND:
  resolved -> unresolved   the base index has a reference here, the candidate has none
  target changed           both have a reference here, and the targets differ
  newly resolved           the candidate has a reference here, the base has none (not an error)
For each of the first two it also says whether scip-python agrees with the base, the candidate,
both, or neither. A location prints as `doc:LINE:COL`: LINE is 1-based, COL is a 0-based code
point offset. The allowlist of gate.py uses the same format.
`--out PREFIX` writes PREFIX.changes.tsv, and PREFIX.multi.tsv when the candidate has occurrences with
several symbols. A row of the multi file holds the location, `union` or the reason that the set is
not a union (see owners.py), `sp=covered`, `sp=not_covered`, `sp_none`, or `sp_external`, and the
symbols. Import this module for `load_positions` and `diff`.
"""
import collections
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import compare  # noqa: E402
import oraclelib  # noqa: E402
import owners  # noqa: E402


def sym_of(key):
    return key if isinstance(key, str) else key[1]


def target_set(refs_at, defs):
    """Frozenset of target tokens for the reference entries at one position."""
    out = set()
    for _, key, _ in refs_at:
        if key in defs:
            for doc, rng in defs[key]:
                out.add(("def", doc, rng[0], rng[1]))
        else:
            out.add(("ext", sym_of(key)))
    return frozenset(out)


def load_positions(index_path, root):
    """Return (targets, multi, defs, refs).

    targets: {(doc, (line0, col)): frozenset of tokens}.
    multi: [(doc, range, sorted symbols)] for occurrences that have more than one symbol.
    """
    _, defs, refs, _ = compare.load(index_path, root)
    targets = {pos: target_set(lst, defs) for pos, lst in refs.items()}
    multi = []
    for (doc, _), lst in refs.items():
        by_range = collections.defaultdict(set)
        for rng, key, _ in lst:
            by_range[rng].add(sym_of(key))
        for rng, syms in by_range.items():
            if len(syms) > 1:
                multi.append((doc, rng, sorted(syms)))
    return targets, multi, defs, refs


def fmt(pos):
    return "%s:%d:%d" % (pos[0], pos[1][0] + 1, pos[1][1])


def diff(base, cand):
    """Return {"lost": [pos], "changed": [pos], "gained": [pos]} for two `targets` dicts."""
    out = {"lost": [], "changed": [], "gained": []}
    for pos, tb in base.items():
        tc = cand.get(pos)
        if tc is None:
            out["lost"].append(pos)
        elif tc != tb:
            out["changed"].append(pos)
    for pos in cand:
        if pos not in base:
            out["gained"].append(pos)
    for v in out.values():
        v.sort()
    return out


def locs_of(tokens):
    return [(t[1], (t[2], t[3], t[2], t[3])) for t in tokens if t[0] == "def"]


def sp_agreement(pos, base_t, cand_t, ref_targets):
    """Say which of base and candidate agrees with scip-python at a position."""
    rt = ref_targets.get(pos)
    if not rt:
        return "sp_none"
    r = locs_of(rt)
    if not r:
        return "sp_external"
    ab = compare.any_match(locs_of(base_t.get(pos, ())), r, compare.overlap)
    ac = compare.any_match(locs_of(cand_t.get(pos, ())), r, compare.overlap)
    return "sp=both" if ab and ac else "sp=base_only" if ab else "sp=cand_only" if ac else "sp=neither"


def main():
    a = sys.argv[1:]
    out = None
    limit = 20
    for flag in ("--out", "--limit"):
        if flag in a:
            i = a.index(flag)
            val = a[i + 1]
            del a[i:i + 2]
            if flag == "--out":
                out = val
            else:
                limit = int(val)
    if len(a) != 3:
        raise SystemExit(__doc__)
    corpus, bdir, cdir = a
    root = oraclelib.corpus_root(corpus)
    bt, _, _, _ = load_positions(oraclelib.scippy_index(bdir), root)
    ct, multi, _, _ = load_positions(oraclelib.scippy_index(cdir), root)
    rt, _, _, _ = load_positions(oraclelib.reference_index(corpus, cdir), root)
    d = diff(bt, ct)
    print("locations: base %d, candidate %d; lost %d, changed %d, gained %d; multi-symbol occurrences in candidate %d" % (
        len(bt), len(ct), len(d["lost"]), len(d["changed"]), len(d["gained"]), len(multi)))
    rows = []
    for kind in ("lost", "changed", "gained"):
        agree = collections.Counter()
        for pos in d[kind]:
            ag = sp_agreement(pos, bt, ct, rt) if kind != "gained" else ""
            agree[ag] += 1
            rows.append((kind, fmt(pos), ag))
        print("%-8s %6d  %s" % (kind, len(d[kind]), dict(agree) if kind != "gained" else ""))
        for pos in d[kind][:limit]:
            print("    " + fmt(pos))
    if multi:
        mrows = []
        for doc, rng, syms in sorted(multi):
            pos = (doc, tuple(rng[:2]))
            rt_ = rt.get(pos)
            if not rt_:
                cov = "sp_none"
            elif not locs_of(rt_):
                cov = "sp_external"
            else:
                cov = "sp=covered" if compare.any_match(locs_of(ct.get(pos, ())), locs_of(rt_), compare.overlap) else "sp=not_covered"
            mrows.append((fmt(pos), owners.set_problem(syms) or "union", cov, " | ".join(syms)))
        kinds = collections.Counter((m[1] if m[1] == "union" else "not a union", m[2]) for m in mrows)
        print("multi-symbol occurrences: %s" % dict(kinds))
        for m in mrows[:limit]:
            print("    " + "  ".join(m[:3]))
    else:
        mrows = []
    if out:
        with open(out + ".changes.tsv", "w") as f:
            for r in rows:
                f.write("\t".join(r) + "\n")
        if mrows:
            with open(out + ".multi.tsv", "w") as f:
                for r in mrows:
                    f.write("\t".join(r) + "\n")


if __name__ == "__main__":
    main()
