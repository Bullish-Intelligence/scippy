"""usage: python3 -I caseblocks.py ROOT SCIPPY.scip SCIPPY_PYTHON.scip
Count references on the lines of case clauses (pattern, guard, and body)."""
import ast, os, sys
sys.path.insert(0, '/home/andrew/Projects/scippy/scripts/oracle')
from scipread import read_index
root, a, b = sys.argv[1:4]
sc = read_index(a)['documents']; sp = read_index(b)['documents']
tot_sp = tot_sc = tot_files = ncases = 0
def refs(doc):
    out = {}
    for r, s, ro in doc['occs']:
        if not ro & 1 and s:
            out.setdefault(r[0], set()).add((r[1], s))
    return out
for path, d in sp.items():
    full = os.path.join(root, path)
    try: tree = ast.parse(open(full, encoding='utf-8', errors='replace').read())
    except Exception: continue
    lines = set()
    for n in ast.walk(tree):
        if isinstance(n, ast.match_case):
            ncases += 1
            end = max(getattr(x, 'end_lineno', 0) for x in n.body)
            for l in range(n.pattern.lineno, end + 1): lines.add(l - 1)
    if not lines: continue
    tot_files += 1
    rsp = refs(d); rsc = refs(sc.get(path, {'occs': []}))
    for l in lines:
        for c, s in rsp.get(l, ()):
            tot_sp += 1
            if any(cc == c for cc, _ in rsc.get(l, ())): tot_sc += 1
print('files %d cases %d | scip-python refs in case clauses %d | scippy resolves %d (%.2f%%)' % (tot_files, ncases, tot_sp, tot_sc, 100.0 * tot_sc / max(tot_sp, 1)))
