"""Accuracy oracle: compare Scippy against scip-python by definition location.

Usage: python3 -I compare.py CORPUS_ROOT SCIPPY.scip SCIPPYTHON.scip OUTDIR

Writes OUTDIR/summary.json and OUTDIR/disagreements.jsonl.
"""
import bisect
import collections
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from scipread import read_index  # noqa: E402
import ast  # noqa: E402
import cause  # noqa: E402

# Stub sources of the Stack Graphs fork. They give the star parameters of external symbols.
# Set STACK_GRAPHS_DIR to a checkout of the fork at the pin. Default: ~/Projects/stack-graphs.
FORK_PY = os.path.join(
    os.environ.get("STACK_GRAPHS_DIR", os.path.expanduser("~/Projects/stack-graphs")),
    "languages/tree-sitter-stack-graphs-python/src",
)
STUBS = {"builtins/": "builtins.py", "typing/": "stubs/typing.py"}


def star_params():
    """Descriptor suffixes, such as `typing/NamedTuple#__init__().(args)`, of `*args` and `**kwargs`."""
    out = set()

    def walk(node, prefix):
        for ch in ast.iter_child_nodes(node):
            if isinstance(ch, ast.ClassDef):
                walk(ch, prefix + ch.name + "#")
            elif isinstance(ch, (ast.FunctionDef, ast.AsyncFunctionDef)):
                for a in (ch.args.vararg, ch.args.kwarg):
                    if a is not None:
                        out.add("%s%s().(%s)" % (prefix, ch.name, a.arg))
                walk(ch, prefix + ch.name + "().")
            elif not isinstance(ch, ast.Lambda):
                walk(ch, prefix)

    for mod, f in STUBS.items():
        try:
            walk(ast.parse(open(os.path.join(FORK_PY, f)).read()), mod)
        except (OSError, SyntaxError):
            pass
    return out


STAR = star_params()

DEF, IMPORT, WRITE, READ = 1, 2, 4, 8


def is_local(sym):
    return sym.startswith("local ")


def to_cp(line, col, enc):
    """Convert a column in the document encoding to a code point index."""
    if line.isascii():
        return col
    if enc == 1:  # UTF-8 bytes
        return len(line.encode("utf-8")[:col].decode("utf-8", "ignore"))
    if enc == 2:  # UTF-16 code units
        return len(line.encode("utf-16-le")[:col * 2].decode("utf-16-le", "ignore"))
    return col


def load(path, root):
    ix = read_index(path)
    for doc, d in ix["documents"].items():
        try:
            lines = open(os.path.join(root, doc), encoding="utf-8", errors="replace").read().split("\n")
        except OSError:
            continue
        out = []
        for rng, sym, roles in d["occs"]:
            if len(rng) == 4 and rng[0] < len(lines) and rng[2] < len(lines):
                rng = (rng[0], to_cp(lines[rng[0]], rng[1], d["enc"]), rng[2], to_cp(lines[rng[2]], rng[3], d["enc"]))
            out.append((rng, sym, roles))
        d["occs"] = out
    defs = collections.defaultdict(list)  # symbol key -> [(doc, range)]
    refs = collections.defaultdict(list)  # (doc, start) -> [(range, symkey, roles)]
    defpos = collections.defaultdict(list)  # (doc, start) -> [(range, symkey)]
    for doc, d in ix["documents"].items():
        for rng, sym, roles in d["occs"]:
            if not sym:
                continue
            key = (doc, sym) if is_local(sym) else sym
            if roles & DEF:
                defs[key].append((doc, rng))
                defpos[(doc, rng[:2])].append((rng, key))
            else:
                refs[(doc, rng[:2])].append((rng, key, roles))
    return ix, defs, refs, defpos


def overlap(a, b):
    """a, b = (doc, rng). Same doc and the ranges overlap, or have the same start."""
    if a[0] != b[0]:
        return False
    ra, rb = a[1], b[1]
    if ra[:2] == rb[:2]:
        return True
    return ra[:2] < (rb[2], rb[3]) and rb[:2] < (ra[2], ra[3])


def strict(a, b):
    return a[0] == b[0] and a[1][:2] == b[1][:2]


def any_match(A, B, fn):
    for a in A:
        for b in B:
            if fn(a, b):
                return True
    return False


def base_name(sym):
    """Descriptor name at the end of a symbol, for same-name checks."""
    s = sym.rstrip(".#:()/[]!")
    m = re.search(r"([A-Za-z_0-9]+)\W*$", sym)
    return m.group(1) if m else s


def builtin_name(sym):
    """Return the builtins descriptor path, or None if the symbol is not a builtin."""
    m = re.search(r"\bbuiltins/(.*)$", sym)
    if not m:
        return None
    if "python-stdlib" not in sym:
        return None
    rest = m.group(1)
    rest = rest.replace("__init__:", "")
    return rest


def main():
    root, scippy_p, sp_p, outdir = sys.argv[1:5]
    os.makedirs(outdir, exist_ok=True)
    A_ix, A_defs, A_refs, A_defpos = load(scippy_p, root)  # Scippy
    B_ix, B_defs, B_refs, B_defpos = load(sp_p, root)  # scip-python
    docs_a = set(A_ix["documents"])
    docs_b = set(B_ix["documents"])
    print("tools", A_ix["tool"], B_ix["tool"])
    print("docs", len(docs_a), len(docs_b), "common", len(docs_a & docs_b))

    src_cache = {}

    def src_line(doc, line):
        if doc not in src_cache:
            try:
                src_cache[doc] = open(os.path.join(root, doc), encoding="utf-8", errors="replace").read().split("\n")
            except OSError:
                src_cache[doc] = []
        ls = src_cache[doc]
        return ls[line] if line < len(ls) else ""

    def text_at(doc, rng):
        if rng[0] != rng[2]:
            return ""
        return src_line(doc, rng[0])[rng[1]:rng[3]]

    def prev_char(doc, rng):
        l = src_line(doc, rng[0])
        return l[rng[1] - 1] if rng[1] > 0 else ""

    def in_import_module_name(doc, rng):
        """True if the range is a component of a module name in an import statement.

        Matches `import a.b.c` (any component) and `from a.b import x` (any
        component of `a.b`). Looks at the first line of the statement only.
        """
        if rng[0] != rng[2]:
            return False
        line = src_line(doc, rng[0])
        pre = line[:rng[1]]
        post = line[rng[3]:]
        if re.match(r"\s*import\s+(?:[\w.]+(?:\s+as\s+\w+)?\s*,\s*)*[\w.]*$", pre):
            return True
        if re.match(r"\s*from\s+[\w.]*$", pre):
            return bool(re.match(r"[\w.]*\s+import\b", post))
        return False

    def is_import_prefix(doc, rng, ra, ta, tb):
        """Both plausible: import prefix.

        `import a.b.c` binds the name `a` to package `a`. Scippy points the
        leading name at package `a`. scip-python points it at the submodule.
        Both readings are valid. Returns True when the position is a module-name
        component in an import statement and either
          - Scippy's target is a package (`a/__init__.py`) that is a path prefix of
            a scip-python target module, or
          - (tb is None) scip-python has no occurrence and Scippy emits a module symbol.
        """
        if not any(roles & IMPORT for _, _, roles in ra):
            return False
        if not in_import_module_name(doc, rng):
            return False
        if tb is None:
            return any((k if isinstance(k, str) else k[1]).endswith("/") for _, k, _ in ra)
        for d, _ in ta:
            if not d.endswith("/__init__.py") and d != "__init__.py":
                continue
            pkg = d[: -len("__init__.py")]
            for d2, _ in tb:
                if d2 != d and d2.startswith(pkg):
                    return True
        return False

    def in_project(key, defs):
        return key in defs

    def targets(rec_list, defs):
        """Definition locations (in project) for a list of (range, symkey, roles)."""
        out = []
        ext = False
        for rng, key, roles in rec_list:
            if key in defs:
                out.extend(defs[key])
            else:
                ext = True
        return out, ext

    # Normalise Scippy and scip-python import aliases: if a def location of a
    # tool is itself a non-def reference position of that tool, follow it.
    def follow(tdefs, trefs, locs):
        res = []
        for loc in locs:
            seen = 0
            cur = [loc]
            while seen < 5:
                nxt = []
                changed = False
                for c in cur:
                    rs = trefs.get((c[0], c[1][:2]))
                    if rs and not any(True for _ in []):
                        # an import alias: def occurrence position that is also a reference
                        t = []
                        for rng, key, roles in rs:
                            if key in tdefs:
                                t.extend(tdefs[key])
                        t = [x for x in t if x != c]
                        if t:
                            nxt.extend(t)
                            changed = True
                            continue
                    nxt.append(c)
                cur = nxt
                seen += 1
                if not changed:
                    break
            res.extend(cur)
        return res

    def is_keyword(doc, rng):
        fi = cause.info(root, doc)
        return (rng[0], fi.bcol(rng[0], rng[1])) in fi.kwarg

    def is_star_param(key, defs):
        """True if a Scippy symbol is a `*args` or `**kwargs` parameter."""
        if key in defs:
            for d, r in defs[key]:
                l = src_line(d, r[0])
                if r[1] > 0 and l[r[1] - 1] == "*":
                    return True
            return False
        sym = key if isinstance(key, str) else key[1]
        return any(sym.endswith(" " + x) or sym.endswith("/" + x) for x in STAR)

    def star_keyword(doc, rng, ra):
        """A keyword argument that Scippy resolves to a star parameter only. It counts as wrong."""
        return bool(ra) and is_keyword(doc, rng) and all(is_star_param(k, A_defs) for _, k, _ in ra)

    # Reference positions (doc, start) in each tool.
    pos_a = set(A_refs)
    pos_b = set(B_refs)

    stats = collections.Counter()
    cat_stats = collections.defaultdict(collections.Counter)
    dis = []
    plaus_pos = set()  # (doc, start) positions in the "both plausible: import prefix" class

    def category(doc, rng, roles_union):
        t = text_at(doc, rng)
        pc = prev_char(doc, rng)
        if roles_union & IMPORT:
            return "import"
        if t in ("self", "cls"):
            return "self_cls"
        if pc == ".":
            return "attribute"
        return "name"

    # Per position evaluation.
    for pos in sorted(pos_a | pos_b):
        ra = A_refs.get(pos, [])
        rb = B_refs.get(pos, [])
        doc = pos[0]
        ta, ea = targets(ra, A_defs)
        tb, eb = targets(rb, B_defs)
        rng = (ra or rb)[0][0]
        roles = 0
        for _, _, r in ra + rb:
            roles |= r
        cat = category(doc, rng, roles)
        has_a = bool(ta)
        has_b = bool(tb)
        if has_b:
            stats["sp_inproj"] += 1
            cat_stats[cat]["sp_inproj"] += 1
        if has_a:
            stats["sc_inproj"] += 1
            cat_stats[cat]["sc_inproj"] += 1
        if has_a and not rb:
            # Coverage: Scippy ref where scip-python has no reference
            kind = "sp_def_here" if pos in B_defpos else "sp_nothing"
            stats["coverage_" + kind] += 1
            cat_stats[cat]["coverage"] += 1
            if is_import_prefix(doc, rng, ra, ta, None):
                stats["coverage_import_prefix"] += 1
                plaus_pos.add((doc, tuple(rng[:2])))
            if len(dis) < 10**7:
                dis.append(("sp_missing_" + kind, cat, doc, rng, ra, rb, ta, tb))
            continue
        if has_a and rb and not has_b:
            stats["coverage_sp_external"] += 1
            dis.append(("sp_missing_external", cat, doc, rng, ra, rb, ta, tb))
            cat_stats[cat]["coverage_ext"] += 1
            continue
        if has_b and not ra and pos in A_defpos:
            # Rebind credit: Scippy has a definition here, scip-python a reference. The symbol of
            # the Scippy definition has the definition location of the scip-python target.
            locs = [loc for _, key in A_defpos[pos] for loc in A_defs[key]]
            if any_match(locs, tb, overlap) or any_match(follow(A_defs, A_refs, locs), follow(B_defs, B_refs, tb), overlap):
                for k in ("both", "sc_inproj", "match_overlap", "match_strict", "match_overlap_follow", "match_strict_follow", "rebind_credit"):
                    stats[k] += 1
                for k in ("both", "sc_inproj", "match_overlap", "match_strict"):
                    cat_stats[cat][k] += 1
                continue
        if has_b and not has_a and star_keyword(doc, rng, ra):
            # Scippy names a star parameter (here external) for a keyword: a wrong target.
            stats["both"] += 1
            stats["sc_inproj"] += 1
            stats["star_keyword"] += 1
            cat_stats[cat]["both"] += 1
            cat_stats[cat]["different"] += 1
            dis.append(("different", cat, doc, rng, ra, rb, ta, tb))
            continue
        if has_b and not has_a:
            # recall miss
            how = "sc_no_occ" if not ra else "sc_external_or_nodef"
            if not ra and pos in A_defpos:
                how = "sc_def_here"  # sp says reference, Scippy says definition (rebinding)
            stats["miss_" + how] += 1
            cat_stats[cat]["miss"] += 1
            dis.append(("sc_missing_" + how, cat, doc, rng, ra, rb, ta, tb))
            continue
        if has_a and has_b:
            stats["both"] += 1
            cat_stats[cat]["both"] += 1
            m_ov = any_match(ta, tb, overlap)
            m_st = any_match(ta, tb, strict)
            fa = follow(A_defs, A_refs, ta)
            fb = follow(B_defs, B_refs, tb)
            m_ov_f = any_match(fa, fb, overlap)
            m_st_f = any_match(fa, fb, strict)
            if m_ov:
                stats["match_overlap"] += 1
                cat_stats[cat]["match_overlap"] += 1
            if m_st:
                stats["match_strict"] += 1
                cat_stats[cat]["match_strict"] += 1
            if m_ov_f:
                stats["match_overlap_follow"] += 1
            if m_st_f:
                stats["match_strict_follow"] += 1
            if m_ov_f and star_keyword(doc, rng, ra):
                stats["star_keyword_agreed"] += 1
            if not m_ov_f:
                if star_keyword(doc, rng, ra):
                    stats["star_keyword"] += 1
                if is_import_prefix(doc, rng, ra, ta, tb):
                    stats["different_import_prefix"] += 1
                    plaus_pos.add((doc, tuple(rng[:2])))
                dis.append(("different", cat, doc, rng, ra, rb, ta, tb))
                cat_stats[cat]["different"] += 1

    # Builtins by name, per position.
    def builtin_map(refs):
        m = {}
        for pos, lst in refs.items():
            for rng, key, roles in lst:
                if isinstance(key, str):
                    b = builtin_name(key)
                    if b:
                        m.setdefault(pos, set()).add(b)
        return m

    bm_a = builtin_map(A_refs)
    bm_b = builtin_map(B_refs)
    bstats = collections.Counter()
    bdis = []
    for pos in set(bm_a) | set(bm_b):
        if pos in bm_a and pos in bm_b:
            bstats["both"] += 1
            if bm_a[pos] & bm_b[pos]:
                bstats["same_name"] += 1
            else:
                bstats["different"] += 1
                bdis.append((pos, sorted(bm_a[pos]), sorted(bm_b[pos])))
        elif pos in bm_a:
            bstats["scippy_only"] += 1
            if pos in B_refs:
                bstats["scippy_only_sp_has_other_ref"] += 1
            bdis.append((pos, sorted(bm_a[pos]), None))
        else:
            bstats["sp_only"] += 1
            bdis.append((pos, None, sorted(bm_b[pos])))
    bnames_a = collections.Counter(n for s in bm_a.values() for n in s)
    bnames_b = collections.Counter(n for s in bm_b.values() for n in s)

    summary = {
        "tools": [A_ix["tool"], B_ix["tool"]],
        "docs": [len(docs_a), len(docs_b), len(docs_a & docs_b)],
        "ref_positions": [len(pos_a), len(pos_b)],
        "stats": dict(stats),
        "by_category": {k: dict(v) for k, v in cat_stats.items()},
        "builtins": dict(bstats),
        "builtin_names_scippy_total": sum(bnames_a.values()),
        "builtin_names_sp_total": sum(bnames_b.values()),
    }
    json.dump(summary, open(os.path.join(outdir, "summary.json"), "w"), indent=1)
    with open(os.path.join(outdir, "disagreements.jsonl"), "w") as f:
        for kind, cat, doc, rng, ra, rb, ta, tb in dis:
            f.write(json.dumps({
                "kind": kind, "cat": cat, "doc": doc, "rng": rng, "text": text_at(doc, rng),
                "plaus": "import_prefix" if (doc, tuple(rng[:2])) in plaus_pos else "",
                "star_kw": kind == "different" and star_keyword(doc, rng, ra),
                "sc": [[k if isinstance(k, str) else k[1], r] for _, k, r in ra],
                "sp": [[k if isinstance(k, str) else k[1], r] for _, k, r in rb],
                "sc_def": [[d, list(r)] for d, r in ta[:4]],
                "sp_def": [[d, list(r)] for d, r in tb[:4]],
            }) + "\n")
    with open(os.path.join(outdir, "builtin_disagreements.jsonl"), "w") as f:
        for pos, a, b in bdis:
            f.write(json.dumps({"doc": pos[0], "start": pos[1], "sc": a, "sp": b}) + "\n")
    print(json.dumps(summary, indent=1))


if __name__ == "__main__":
    main()
