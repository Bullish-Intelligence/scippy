"""Show samples. usage: show.py CORPUS_ROOT DISAGREEMENTS.jsonl KIND CAT [N] [SKIP] [filter-substring-of-text]"""
import json, sys, os, random
root, f, kind, cat = sys.argv[1:5]
n = int(sys.argv[5]) if len(sys.argv) > 5 else 5
skip = int(sys.argv[6]) if len(sys.argv) > 6 else 0
flt = sys.argv[7] if len(sys.argv) > 7 else None
cache = {}
def line(doc, l):
    if doc not in cache:
        cache[doc] = open(os.path.join(root, doc), encoding='utf-8', errors='replace').read().split('\n')
    return cache[doc][l].strip()[:150]
rows = [json.loads(l) for l in open(f)]
rows = [r for r in rows if r['kind'] == kind and (cat == '*' or r['cat'] == cat) and (not flt or flt in r['text'])]
random.Random(1).shuffle(rows)
print(len(rows), 'rows')
for r in rows[skip:skip + n]:
    print('---', r['doc'], r['rng'][0] + 1, r['text'], '|', line(r['doc'], r['rng'][0]))
    print('  sc:', r['sc'], ' sp:', r['sp'])
    for d, rg in r['sc_def'][:2]: print('  sc_def', d, rg[0] + 1, line(d, rg[0]))
    for d, rg in r['sp_def'][:2]: print('  sp_def', d, rg[0] + 1, line(d, rg[0]))
