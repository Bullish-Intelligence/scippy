"""Final metrics from summary.json and buckets.json. usage: metrics.py CMPDIR"""
import json, sys, re, os
d = sys.argv[1]
s = json.load(open(os.path.join(d, 'summary.json')))['stats']
b = json.load(open(os.path.join(d, 'buckets.json')))
def cnt(prefix): return sum(v['count'] for k, v in b.items() if k.startswith(prefix))
sp = s['sp_inproj']; sc = s['sc_inproj']; both = s['both']; m = s['match_overlap']; ms = s['match_strict']
plaus = cnt('both_plausible'); wrong = cnt('scippy_wrong')
miss = cnt('scippy_missing')
design = {k: v['count'] for k, v in b.items() if k.startswith('scippy_missing') and re.search(r'C1_keyword|C_rebind|C_file_skipped', k)}
dsum = sum(design.values())
cov_none = s.get('coverage_sp_nothing', 0); cov_ext = s.get('coverage_sp_external', 0)
print('sp in-project positions', sp, '| scippy in-project positions', sc, '| both', both)
print('precision overlap %.2f%% (%d/%d), strict %.2f%% (%d)' % (100*m/both, m, both, 100*ms/both, ms))
print('precision, plausible counted correct: %.2f%%; wrong=%d (%.3f%%)' % (100*(m+plaus)/both, wrong, 100*wrong/both))
print('raw recall (rebind credit counted matched) %.2f%% (%d/%d), strict %.2f%%' % (100*m/sp, m, sp, 100*ms/sp))
print('rebind credit %d; rebind not credited (C_rebind) %d' % (s.get('rebind_credit', 0), s.get('miss_sc_def_here', 0)))
print('raw recall + plausible %.2f%%' % (100*(m+plaus)/sp))
print('keyword to star parameter (wrong): %d' % s.get('star_keyword', 0))
fam = lambda pat: sum(v['count'] for k, v in b.items() if k.startswith('scippy_missing') and re.search(pat, k))
print('classes: receiver type (C4/C5) %d [sp coarse %d, keyword method param %d, attribute store %d]; synthesized __init__ %d; C1 keyword rest %d; C_rebind %d' % (
    fam(r'/ C4|/ C5'), fam('C4c_'), fam('C4k_'), fam('C5s_'), fam('C_synth_init'), fam('C1_keyword'), fam('C_rebind')))
rest_den = sp - dsum - (cnt('both_plausible / sp_points_at_owner'))
print('design-gap missing:', dsum, design)
print('recall without design gaps: %.2f%% (%d/%d)' % (100*(m+plaus)/(sp-dsum), m+plaus, sp-dsum))
art = cnt('import_prefix_artifact'); art_diff = s.get('different_import_prefix', 0)
print('coverage (raw): scippy refs w/o sp ref: %d none + %d sp external = %d (%.2f%% of Scippy in-project positions %d)' % (cov_none, cov_ext, cov_none+cov_ext, 100*(cov_none+cov_ext)/sc, sc))
print('coverage (without the import-prefix artifact): %d - %d = %d (%.2f%%)' % (cov_none+cov_ext, art, cov_none+cov_ext-art, 100*(cov_none+cov_ext-art)/sc))
print('import-prefix class: %d different (in plausible) + %d no-sp-occurrence (in coverage)' % (art_diff, art))
print('precision without the import-prefix class counted correct = raw precision above; plausible total %d (of which import prefix %d)' % (plaus, art_diff))
print('builtins', json.load(open(os.path.join(d, 'summary.json')))['builtins'])
w4 = cnt('both_plausible / import_prefix_at_usage_site')
print('import-prefix class at usage sites (was W4, now in plausible, decision 2026-11-09): %d; import statements: %d' % (w4, art_diff))
print('wrong targets (W4 no longer counted wrong): %d (%.3f%%)' % (wrong, 100*wrong/both))
