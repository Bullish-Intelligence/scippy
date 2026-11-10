"""Classify disagreements into buckets and causes.

Usage: python3 -I analyze.py CORPUS_ROOT CMPDIR SCIPPY_SCIP TRACE OUT_JSON
Reads CMPDIR/disagreements.jsonl. Writes buckets (count, 3 samples each) and cause tables.
"""
import collections
import json
import os
import random
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import cause  # noqa: E402
from scipread import read_index  # noqa: E402

root, cmpdir, scippy_p, trace_p, out_p = sys.argv[1:6]
rows = [json.loads(l) for l in open(os.path.join(cmpdir, "disagreements.jsonl"))]
SC_DOCS = set(read_index(scippy_p)["documents"])

trace = {}
for l in open(trace_p, encoding="utf-8", errors="replace"):
    parts = l.rstrip("\n").split("\t")
    if len(parts) < 3:
        continue
    m = re.match(r"(.*):(\d+):(\d+)$", parts[0])
    if not m:
        continue
    outcome = parts[1]
    reason = "resolved" if outcome == "resolved" else (parts[2] if outcome == "unresolved" else outcome)
    trace[(m.group(1), int(m.group(2)) - 1, int(m.group(3)))] = reason


def line_at(doc, ln):
    fi = cause.info(root, doc)
    return fi.lines[ln].strip()[:140] if ln < len(fi.lines) else ""


def trace_reason(doc, rng):
    fi = cause.info(root, doc)
    return trace.get((doc, rng[0], fi.bcol(rng[0], rng[1])), "no_reference_node")


def sp_kind(sym):
    if sym.startswith("local "):
        return "local"
    if sym.endswith(":"):
        return "module"
    if sym.endswith("#"):
        return "class"
    if re.search(r"\)\.\([^)]*\)$", sym):
        return "param"
    if sym.endswith(")."):
        return "func/method"
    if sym.endswith("."):
        return "var/attr"
    return "other"


def sc_kind(sym):
    if sym.startswith("local "):
        return "local"
    d = sym.split(" . ", 1)[-1]
    return "class" if d.endswith("#") else "func/method" if d.endswith(").") else "var/attr" if d.endswith(".") else "other"


def sp_class_path(sym):
    m = re.search(r"`([^`]+)`/(.*)$", sym)
    return m.group(1).replace(".", "/") + "/" + m.group(2) if m else None


def member_name(sym):
    m = re.search(r"([A-Za-z_0-9]+)(\(\)\.|\.)$", sym)
    return m.group(1) if m else None


def miss_cause(r):
    """Primary cause tag for a Scippy miss."""
    sp0 = r["sp"][0][0]
    sk = sp_kind(sp0)
    fi = cause.info(root, r["doc"])
    ln, col = r["rng"][0], r["rng"][1]
    b = fi.bcol(ln, col)
    reason = trace_reason(r["doc"], r["rng"])
    if r["kind"] == "sc_missing_sc_def_here":
        return "C_rebind: Scippy emits a definition, scip-python a reference (role difference)"
    if r["doc"] not in SC_DOCS:
        return "C_file_skipped: bind error skips the whole file (x = yield ... rule)"
    if sk == "module":
        return "C_module_ref: reference to a module (no occurrence by design)"
    if r["cat"] == "attribute" and sk == "class":
        return "C4c_receiver_type_sp_coarse: scip-python points at the owner class"
    if sk in ("param", "var/attr") and (ln, b) in fi.kwarg and r["cat"] in ("name", "self_cls"):
        if sk == "var/attr":
            return "C_synth_init_field: keyword names a dataclass or NamedTuple field (synthesized __init__)"
        if re.search(r"#[^#/]*\(\)\.\([^)]*\)$", sp0):
            return "C4k_receiver_type_keyword_method_param: keyword names a parameter of a method"
        return "C1_keyword_argument_name: `f(name=v)` has no reference node"
    if (ln, b) in fi.store_attrs or (ln, b) in fi.aug:
        return "C5s_receiver_type_attribute_store_target: `obj.attr = v` has no target"
    if fi.in_ranges(fi.ann, ln, col) or fi.in_ranges(fi.strs, ln, col):
        return "C2_annotation_name (forward ref, enclosing class, string): " + reason
    f = fi.enclosing_func(ln)
    dec = bool(f and f[2])
    if r["cat"] == "self_cls":
        if re.search(r"[,(]\s*\*|\*\w", line_at(r["doc"], ln)) or "=" in line_at(r["doc"], ln):
            return "C_self_in_tuple_assignment_target `self.a, self.b = ...`: " + reason
        return "C_self_other: " + reason
    if r["cat"] == "attribute":
        rf = fi.receiver_form(ln, col)
        if dec and rf in ("self.name", "cls.name", "self.attr.name"):
            return "C3_decorated_method_self_or_cls (tsg `method definition` pattern skips decorated_definition)"
        if rf == "local.name":
            org = fi.receiver_origin(ln, col)
            if org.startswith("param_") and "|" in org:
                kind_, ann = org.split("|", 1)
                owner = re.search(r"`/(?:[A-Za-z_0-9]+#)*?([A-Za-z_0-9]+)#", sp0)
                owner = owner.group(1) if owner else ""
                if owner and not re.search(r"\b" + re.escape(owner) + r"\b", ann):
                    return "C4a_receiver_param_declared_type_differs_from_member_owner (isinstance narrowing or subclass member)"
                return "C4b_receiver_param_annotated_but_member_not_found (%s)" % kind_
            return "C4_attribute_on_local_receiver: " + org
        if rf in ("self.attr.name", "local.attr.name", "chain3+"):
            return "C5_attribute_chain (a.b.c): receiver type needs annotation or value flow"
        if rf == "call_or_subscript_result":
            return "C5_attribute_on_call_or_subscript_result"
        if rf == "self.name":
            return "C6_self_attribute_not_found (plain method): " + reason
        return "C_attribute_other: " + rf
    if (ln, b) in fi.aug or (ln, b) in fi.store_names:
        return "C_name_store_target: " + reason
    line = line_at(r["doc"], ln)
    if re.search(r"\*\s*" + re.escape(r["text"]) + r"\b", line) or "*" + r["text"] in line:
        return "C7_starred_target_or_arg `*name`: " + reason
    return "C_name_other (%s): %s" % (sk, reason)


def package_prefix_at_usage_site(r):
    """Same artifact as the import prefix, but at a usage site such as `mypy.nodes.X`.

    Scippy points the name at package `a`; scip-python points it at a submodule of `a`.
    """
    for d, _ in r["sc_def"]:
        if d.endswith("/__init__.py") and any(d2 != d and d2.startswith(d[: -len("__init__.py")]) for d2, _ in r["sp_def"]):
            return True
    return False


def function_type_parameter(r):
    """scip-python gives a type parameter of a function the symbol of the function.

    Scippy gives the type parameter, a local symbol at its declaration in the `def` header.
    """
    if r["cat"] != "name" or not (r["sc"] and r["sp"] and r["sc_def"] and r["sp_def"]):
        return False
    if not r["sc"][0][0].startswith("local ") or not r["sp"][0][0].endswith(")."):
        return False
    (sc_doc, sc_rng), (sp_doc, sp_rng) = r["sc_def"][0], r["sp_def"][0]
    if sc_doc != sp_doc or not 0 <= sc_rng[0] - sp_rng[0] <= 3:
        return False
    return re.match(r"(async\s+)?def\s+\w+\[", line_at(sp_doc, sp_rng[0])) is not None


def wrong_cause(r):
    sc0 = r["sc"][0][0]
    if r.get("star_kw"):
        return "W5_keyword_resolved_to_a_star_parameter (`*args` or `**kwargs`)"
    if r["cat"] == "attribute" and sc_kind(sc0) == "local":
        return "W1_attribute_resolved_to_a_local (value flow through assignment RHS)"
    if sc_kind(sc0) == "local":
        return "W2_name_resolved_to_other_local"
    return "W3_member_other"


buckets = collections.defaultdict(list)
for r in rows:
    k = r["kind"]
    sp0 = r["sp"][0][0] if r["sp"] else ""
    sc0 = r["sc"][0][0] if r["sc"] else ""
    if k == "different" and r.get("plaus") == "import_prefix":
        buckets[("both_plausible", "import_prefix: Scippy points `a` of `import a.b.c` at package a, scip-python at the submodule")].append(r)
    elif k.startswith("sp_missing") and r.get("plaus") == "import_prefix":
        buckets[("import_prefix_artifact", "scip-python has no occurrence for a module-name component in an import statement")].append(r)
    elif k == "different" and r.get("star_kw"):
        buckets[("scippy_wrong", wrong_cause(r))].append(r)
    elif k == "different":
        spc = sp_class_path(sp0)
        sc_d = [s.split(" . ", 1)[-1] for s, _ in r["sc"]]
        if spc and spc.endswith("#") and any(d.startswith(spc) for d in sc_d):
            buckets[("both_plausible", "sp_points_at_owner_class, Scippy at the member")].append(r)
        elif r["cat"] == "attribute" and member_name(sp0) and member_name(sp0) == member_name(sc0.split(" . ", 1)[-1]) == r["text"]:
            buckets[("both_plausible", "same_name_member_in_another_class (inheritance/override)")].append(r)
        elif r["cat"] == "attribute" and spc and spc.endswith("#") and sc_kind(sc0) in ("var/attr", "func/method") and member_name(sc0.split(" . ", 1)[-1]) == r["text"]:
            buckets[("both_plausible", "sp_points_at_a_class_that_is_not_the_owner, Scippy at a same-name member")].append(r)
        elif function_type_parameter(r):
            buckets[("both_plausible", "function_type_parameter: scip-python gives a type parameter of a function the symbol of the function, Scippy the type parameter")].append(r)
        elif r["cat"] == "name" and sc0.endswith("/") and package_prefix_at_usage_site(r):
            # User decision 2026-11-09: W4 counts as "both plausible: import prefix" at usage sites.
            buckets[("both_plausible", "import_prefix_at_usage_site: Scippy points the leading name (`mypy` of `mypy.nodes.X`) at the package, scip-python at a submodule")].append(r)
        else:
            buckets[("scippy_wrong", wrong_cause(r))].append(r)
    elif k.startswith("sc_missing"):
        buckets[("scippy_missing", miss_cause(r))].append(r)
    elif k.startswith("sp_missing"):
        t = r["text"]
        if k == "sp_missing_external":
            sub = "scip-python resolves to a builtin or stdlib symbol for an attribute or local named `%s`" % (t if t in ("type", "tuple", "types", "function") else "other")
            buckets[("scip_python_missing_or_wrong", sub)].append(r)
        else:
            buckets[("scip_python_missing_or_wrong", "no scip-python occurrence")].append(r)

out = {}
rng_ = random.Random(7)
for (b, s), lst in sorted(buckets.items()):
    samples = []
    for r in rng_.sample(lst, min(3, len(lst))):
        samples.append({
            "at": "%s:%d" % (r["doc"], r["rng"][0] + 1), "text": r["text"], "line": line_at(r["doc"], r["rng"][0]),
            "scippy": [x[0].split(" . ", 1)[-1] for x in r["sc"]],
            "scip_python": [x[0].split(" 0.0.0 ", 1)[-1] for x in r["sp"]],
            "sc_def": ["%s:%d %s" % (d, rg[0] + 1, line_at(d, rg[0])) for d, rg in r["sc_def"][:1]],
            "sp_def": ["%s:%d %s" % (d, rg[0] + 1, line_at(d, rg[0])) for d, rg in r["sp_def"][:1]],
        })
    out["%s / %s" % (b, s)] = {"count": len(lst), "samples": samples}
json.dump(out, open(out_p, "w"), indent=1)
tot = collections.Counter()
for (b, s), lst in buckets.items():
    tot[b] += len(lst)
print(dict(tot))
for k, v in sorted(out.items(), key=lambda kv: -kv[1]["count"]):
    print(v["count"], k)
