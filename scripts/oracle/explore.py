"""Count miss causes with trace reasons. usage: explore.py CORPUS OUTDIR

ORACLE_DIR holds the corpora (default ~/.local/state/scippy/oracle). OUTDIR holds CORPUS/cmp.
"""
import sys, json, collections, re, os
here = os.path.dirname(os.path.abspath(__file__))
odir = os.environ.get('ORACLE_DIR', os.path.expanduser('~/.local/state/scippy/oracle'))
corp, out = sys.argv[1], sys.argv[2]
d = os.path.join(out, corp)
sys.argv = ['x', f'{odir}/corpora/{corp}/root', f'{d}/cmp', f'{d}/scippy.scip', f'{d}/scippy.trace', os.devnull]
src = open(os.path.join(here, 'analyze.py')).read().split("buckets = collections")[0]
exec(src)
sys.path.insert(0, here)
import cause
from scipread import read_index
SC_DOCS=set(read_index(f'{d}/scippy.scip')['documents'])
c = collections.Counter()
ex = collections.defaultdict(list)
for r in rows:
    if not r['kind'].startswith('sc_missing'): continue
    reason = trace_reason(r['doc'], r['rng'])
    sp0 = r['sp'][0][0]; sk = sp_kind(sp0)
    fi = cause.info(root, r['doc'])
    ln, col = r['rng'][0], r['rng'][1]
    if r['kind'] == 'sc_missing_sc_def_here': tag = 'rebind'
    elif r['doc'] not in SC_DOCS: tag = 'file_skipped'
    elif sk == 'module': tag = 'module_ref'
    elif r['cat'] == 'attribute' and sk == 'class': tag = 'sp_coarse'
    elif sk == 'param' and (ln, fi.bcol(ln, col)) in fi.kwarg: tag = 'kwarg_name'
    elif (ln, fi.bcol(ln, col)) in fi.aug: tag = 'aug_assign_target'
    elif (ln, fi.bcol(ln, col)) in fi.store_attrs: tag = 'attr_store_target'
    elif (ln, fi.bcol(ln, col)) in fi.store_names: tag = 'name_store_target'
    elif fi.in_ranges(fi.ann, ln, col) or fi.in_ranges(fi.strs, ln, col) : tag = 'annotation'
    else:
        f = fi.enclosing_func(ln)
        dec = 'decorated' if f and f[2] else 'plain'
        rf = fi.receiver_form(ln, col)
        if rf == 'local.name': rf = 'local.name:' + fi.receiver_origin(ln, col)
        tag = f'{r["cat"]}|{sk}|{rf}|{dec}'
    c[(tag, reason)] += 1
    ex[(tag, reason)].append(r)
tot = sum(c.values()); print(tot)
for k, v in c.most_common(45): print(v, k)
json.dump({'|'.join(k): v for k, v in c.items()}, open(f'{d}/cmp/cause_counts.json', 'w'))
import random
want = sys.argv[1:] if False else os.environ.get('WANT')
if want:
    for k in c:
        if k[0] == want.split('|')[0] if False else ('|'.join(k) == want or k[0] == want):
            for r in random.Random(5).sample(ex[k], min(int(os.environ.get('N', '8')), len(ex[k]))):
                print('  ', r['doc'], r['rng'][0]+1, r['text'], '|', line_at(r['doc'], r['rng'][0]), '| sp:', r['sp'][0][0].split(' 0.0.0 ')[-1][:80], '| def:', line_at(r['sp_def'][0][0], r['sp_def'][0][1][0]) if r['sp_def'] else '')
