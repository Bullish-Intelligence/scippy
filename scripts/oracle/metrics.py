"""Named metrics of one oracle run.

usage: python3 -I metrics.py CMPDIR
Reads CMPDIR/summary.json and CMPDIR/buckets.json. Prints the metrics. Writes CMPDIR/metrics.json.

Each named number answers one question. There is no blended score:
  strict recall and precision      start of the target definition equals the start of the scip-python target
  overlap recall and precision     the old raw numbers, kept for continuity
  owner-class coarse               scip-python gives the owner class, Scippy the member
  import-prefix convention         `a` of `import a.b.c` or `a.b.X`: package or submodule
  same-name other class            both give a member of that name, in different classes
  scip-python first binding        scip-python takes the first of two declarations of a name
  real wrong                       none of the above: the wrong-target class of Scippy
"""
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from oraclelib import read_json  # noqa: E402

# Names and bucket prefixes of the classes of `different` positions. Order is the print order.
DIFFERENT_CLASSES = [
    ("owner_class_coarse", ("both_plausible / sp_points_at_owner_class",)),
    ("import_prefix", ("both_plausible / import_prefix",)),  # also matches `import_prefix_at_usage_site`
    ("same_name_other_class", ("both_plausible / same_name_member_in_another_class", "both_plausible / sp_points_at_a_class_that_is_not_the_owner")),
    ("function_type_parameter", ("both_plausible / function_type_parameter",)),
    ("sp_first_binding", ("sp_first_binding / ",)),
    ("real_wrong", ("scippy_wrong / ",)),
]


def count(buckets, prefixes):
    return sum(v["count"] for k, v in buckets.items() if k.startswith(prefixes))


def compute(cmpdir):
    """Return {"named": {...}, "legacy": {...}, "classes": {class: count}} for a comparison dir."""
    s = read_json(os.path.join(cmpdir, "summary.json"))
    st = s["stats"]
    b = read_json(os.path.join(cmpdir, "buckets.json"))
    sp, sc, both = st["sp_inproj"], st["sc_inproj"], st["both"]
    m, ms = st["match_overlap"], st["match_strict"]

    def pct(n, d):
        return round(100.0 * n / d, 4) if d else 0.0

    named = {
        "sp_inproj": sp,
        "sc_inproj": sc,
        "both": both,
        "strict_recall_pct": pct(ms, sp),
        "strict_precision_pct": pct(ms, both),
        "overlap_recall_pct": pct(m, sp),
        "overlap_precision_pct": pct(m, both),
    }
    diff_total = 0
    for name, prefixes in DIFFERENT_CLASSES:
        n = count(b, prefixes)
        diff_total += n
        named[name] = n
        named[name + "_pct_of_both"] = pct(n, both)
    # A position that matches only after alias following is in neither `match_overlap` nor `different`.
    named["match_through_alias_follow"] = both - m - diff_total
    named["sp_builtin_type_attr"] = count(b, ("sp_builtin_type_attr / ",))
    named["scippy_missing"] = count(b, ("scippy_missing / ",))

    plaus = count(b, ("both_plausible",))
    design = sum(v["count"] for k, v in b.items() if k.startswith("scippy_missing") and re.search(r"C1_keyword|C_rebind|C_file_skipped", k))
    legacy = {
        "precision_overlap_pct": pct(m, both),
        "recall_overlap_pct": pct(m, sp),
        "precision_plausible_blended_pct": pct(m + plaus, both),
        "recall_plausible_blended_pct": pct(m + plaus, sp),
        "wrong_legacy": named["real_wrong"] + named["sp_first_binding"],
        "design_gap_missing": design,
        "recall_without_design_gaps_pct": pct(m + plaus, sp - design),
        "coverage_sp_nothing": st.get("coverage_sp_nothing", 0),
        "coverage_sp_external": st.get("coverage_sp_external", 0),
        "rebind_credit": st.get("rebind_credit", 0),
        "star_keyword": st.get("star_keyword", 0),
        "builtins": s["builtins"],
    }
    classes = {k: v["count"] for k, v in sorted(b.items())}
    return {"named": named, "legacy": legacy, "classes": classes}


LABELS = {
    "owner_class_coarse": "owner-class coarse (scip-python gives the class, Scippy the member)",
    "import_prefix": "import-prefix convention (package or submodule)",
    "same_name_other_class": "same-name member in another class",
    "function_type_parameter": "function type parameter (symbol convention)",
    "sp_first_binding": "scip-python first binding (name declared twice)",
    "real_wrong": "real wrong (Scippy)",
}


def render(r):
    n, lg = r["named"], r["legacy"]
    out = []
    out.append("positions: scip-python in-project %d | Scippy in-project %d | both %d" % (n["sp_inproj"], n["sc_inproj"], n["both"]))
    out.append("== named numbers ==")
    out.append("strict recall      %.2f%%   strict precision   %.2f%%" % (n["strict_recall_pct"], n["strict_precision_pct"]))
    out.append("overlap recall     %.2f%%   overlap precision  %.2f%%   (old raw numbers)" % (n["overlap_recall_pct"], n["overlap_precision_pct"]))
    for name, _ in DIFFERENT_CLASSES:
        out.append("%-66s %7d  (%.3f%% of both)" % (LABELS[name], n[name], n[name + "_pct_of_both"]))
    out.append("%-66s %7d" % ("matched only after alias following", n["match_through_alias_follow"]))
    out.append("%-66s %7d" % ("scip-python builtins/type# for an attribute named type", n["sp_builtin_type_attr"]))
    out.append("%-66s %7d" % ("Scippy missing (scip-python has a target, Scippy none)", n["scippy_missing"]))
    out.append("== legacy numbers (continuity with the older tables) ==")
    out.append("raw recall %.2f%%, raw precision %.2f%%" % (lg["recall_overlap_pct"], lg["precision_overlap_pct"]))
    out.append("wrong, old definition (real wrong + first binding): %d" % lg["wrong_legacy"])
    out.append("blended with plausible (do not use alone): recall %.2f%%, precision %.2f%%" % (lg["recall_plausible_blended_pct"], lg["precision_plausible_blended_pct"]))
    out.append("design-gap missing %d, recall without design gaps %.2f%% (blended)" % (lg["design_gap_missing"], lg["recall_without_design_gaps_pct"]))
    out.append("coverage: Scippy refs without a scip-python ref: %d none + %d external" % (lg["coverage_sp_nothing"], lg["coverage_sp_external"]))
    out.append("rebind credit %d, keyword to star parameter %d" % (lg["rebind_credit"], lg["star_keyword"]))
    out.append("builtins %s" % lg["builtins"])
    return "\n".join(out)


def main():
    d = sys.argv[1]
    r = compute(d)
    json.dump(r, open(os.path.join(d, "metrics.json"), "w"), indent=1, sort_keys=True)
    print(render(r))


if __name__ == "__main__":
    main()
