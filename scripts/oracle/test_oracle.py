"""Unit tests of the oracle scripts. They use no corpus and no indexer.

Run: python3 -I scripts/oracle/test_oracle.py
"""
import atexit
import contextlib
import io
import json
import os
import shutil
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
# oraclelib reads ORACLE_DIR when it loads, so set a scratch dir first.
_SCRATCH = tempfile.mkdtemp(prefix="oracle-test-")
atexit.register(shutil.rmtree, _SCRATCH, True)
os.environ["ORACLE_DIR"] = _SCRATCH
os.environ["ORACLE_CSV"] = os.path.join(_SCRATCH, "classes.csv")

import analyze  # noqa: E402
import difftool  # noqa: E402
import gate  # noqa: E402
import metrics  # noqa: E402
import oraclelib  # noqa: E402
import owners  # noqa: E402
import track  # noqa: E402

SP = "scip-python python python-stdlib 3.11 "


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def row(**kw):
    r = {"kind": "different", "cat": "name", "doc": "m.py", "rng": [0, 0, 0, 1], "text": "x", "plaus": "", "star_kw": False,
         "sc": [], "sp": [], "sc_def": [], "sp_def": []}
    r.update(kw)
    return r


class SpClassPath(unittest.TestCase):
    def test_plain_top_level_module(self):
        self.assertEqual(analyze.sp_class_path(SP + "dataclasses/Field#"), "dataclasses/Field#")

    def test_backtick_module_with_dots(self):
        self.assertEqual(analyze.sp_class_path("scip-python python pkg 0.0.0 `importlib.util`/_LazyModule#a()."), "importlib/util/_LazyModule#a().")

    def test_builtins(self):
        self.assertEqual(analyze.sp_class_path(SP + "builtins/type#"), "builtins/type#")

    def test_local_symbol_has_no_path(self):
        self.assertIsNone(analyze.sp_class_path("local 3"))

    def test_owner_class_without_backticks_is_plausible(self):
        # The old regex needed backticks, so this row counted as a wrong target.
        r = row(cat="attribute", text="default", sp=[[SP + "dataclasses/Field#", [0, 0]]],
                sc=[["scip-scippy python x 0.0.0 . dataclasses/Field#default.", [0, 0]]])
        self.assertEqual(analyze.classify(r)[0:2], ("both_plausible", "sp_points_at_owner_class, Scippy at the member"))


class FirstBinding(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="oracle-src-")
        analyze.root = self.dir
        write(os.path.join(self.dir, "m.py"),
              "Enum = Flag = None\n"      # line 1: first declaration of Flag, column 7
              "class Flag:\n"             # line 2: later declaration, column 6
              "    pass\n"
              "def use():\n"
              "    return Flag\n"
              "def inner():\n"
              "    Flag = 1\n"            # line 7: a declaration in another scope
              "    return Flag\n")

    def case(self, sc_def, sp_def):
        return row(text="Flag", sc=[["scip-scippy python p 0.0.0 . m/Flag#", [4, 11]]], sp=[[SP + "m/Flag.", [4, 11]]],
                   sc_def=[["m.py", sc_def]], sp_def=[["m.py", sp_def]])

    def test_first_of_two_declarations(self):
        r = self.case([1, 6, 1, 10], [0, 7, 0, 11])
        self.assertTrue(analyze.sp_first_binding(r))
        self.assertEqual(analyze.classify(r)[0], "sp_first_binding")

    def test_scip_python_declaration_is_later(self):
        r = self.case([0, 7, 0, 11], [1, 6, 1, 10])
        self.assertFalse(analyze.sp_first_binding(r))
        self.assertEqual(analyze.classify(r)[0], "scippy_wrong")

    def test_other_scope_is_not_a_first_binding(self):
        r = self.case([6, 4, 6, 8], [0, 7, 0, 11])
        self.assertFalse(analyze.sp_first_binding(r))

    def test_other_name_is_not_a_first_binding(self):
        r = self.case([1, 6, 1, 10], [0, 0, 0, 4])  # `Enum` is not `Flag`
        self.assertFalse(analyze.sp_first_binding(r))


class SpWrong(unittest.TestCase):
    """Rules of `sp_wrong_cause`: scip-python is the wrong or coarse side."""

    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="oracle-src-")
        analyze.root = self.dir
        write(os.path.join(self.dir, "sw_m.py"),
              "def f(v):\n"
              "    match v:\n"
              "        case P(a=a, b=b):\n"              # line 3: keyword `a` at col 16, capture at col 18
              "            pass\n"
              "        case P(a=a):\n"                   # line 5: the capture of an earlier case is reused
              "            pass\n"
              "    g(a=a)\n"                           # line 7: a call, not a pattern
              "class K:\n"
              "    def m(self):\n"
              "        __class__ = 1\n"                # line 10: `__class__` at col 8
              "        return self.obj\n")             # line 11: `obj` at col 20
        write(os.path.join(self.dir, "sw_n.py"), "import httpx as h\nh = None\n")
        write(os.path.join(self.dir, "sw_u.py"), "from .unix import X\n")

    def pattern(self, rng, sp_def):
        return row(doc="sw_m.py", text="a", rng=rng, sc=[["scip-scippy python p 0.0.0 . m/P#a.", [0, 0]]], sp=[["local 1", [0, 0]]],
                   sc_def=[["sw_m.py", [0, 0, 0, 1]]], sp_def=[["sw_m.py", sp_def]])

    def test_class_pattern_keyword(self):
        r = self.pattern([2, 15, 2, 16], [2, 17, 2, 18])
        self.assertEqual(analyze.classify(r)[0], "sp_wrong")

    def test_class_pattern_keyword_with_a_capture_of_an_earlier_case(self):
        r = self.pattern([4, 15, 4, 16], [2, 17, 2, 18])
        self.assertEqual(analyze.classify(r)[0], "sp_wrong")

    def test_keyword_of_a_call_is_not_a_pattern(self):
        r = self.pattern([6, 6, 6, 7], [6, 8, 6, 9])
        self.assertEqual(analyze.classify(r)[0], "scippy_wrong")

    def test_module_for_a_name(self):
        r = row(doc="sw_n.py", text="h", sc=[["scip-scippy python p 0.0.0 . n/h.", [0, 0]]], sp=[["scip-python python p 0.0.0 `p.n`/__init__:", [0, 0]]],
                sc_def=[["sw_n.py", [1, 0, 1, 1]]], sp_def=[["sw_n.py", [0, 0, 0, 0]]])
        self.assertEqual(analyze.classify(r)[0], "sp_wrong")

    def test_module_of_another_file_is_no_rule(self):
        r = row(doc="sw_n.py", text="h", sc=[["scip-scippy python p 0.0.0 . n/h.", [0, 0]]], sp=[["scip-python python p 0.0.0 `p.m`/__init__:", [0, 0]]],
                sc_def=[["sw_n.py", [1, 0, 1, 1]]], sp_def=[["sw_m.py", [0, 0, 0, 0]]])
        self.assertEqual(analyze.classify(r)[0], "scippy_wrong")

    def test_module_for_a_name_needs_an_import_binding(self):
        r = row(doc="sw_m.py", text="h", sc=[["scip-scippy python p 0.0.0 . m/h.", [0, 0]]], sp=[["scip-python python p 0.0.0 `p.m`/__init__:", [0, 0]]],
                sc_def=[["sw_m.py", [1, 0, 1, 1]]], sp_def=[["sw_m.py", [0, 0, 0, 0]]])
        self.assertEqual(analyze.classify(r)[0], "scippy_wrong")

    def test_local_class_owner(self):
        r = row(cat="attribute", doc="sw_m.py", text="obj", rng=[10, 20, 10, 23], sc=[["local 4", [0, 0]]],
                sp=[["scip-python python p 0.0.0 `p.m`/K#", [0, 0]]], sc_def=[["sw_m.py", [10, 8, 10, 11]]], sp_def=[["sw_m.py", [7, 6, 7, 7]]])
        self.assertEqual(analyze.classify(r)[0], "sp_wrong")
        r["sp_def"] = [["sw_m.py", [0, 4, 0, 5]]]  # a line that is no class
        self.assertEqual(analyze.classify(r)[0], "scippy_wrong")

    def test_import_names_the_module_that_scippy_gives(self):
        r = row(cat="import", doc="sw_u.py", text="X", rng=[0, 17, 0, 18], sc=[["scip-scippy python p 0.0.0 . unix/X.", [0, 0]]],
                sp=[["scip-python python p 0.0.0 `p.win`/X.", [0, 0]]], sc_def=[["unix.py", [3, 0, 3, 1]]], sp_def=[["win.py", [3, 0, 3, 1]]])
        self.assertEqual(analyze.classify(r)[0], "sp_wrong")
        r["sp_def"] = [["other.py", [3, 0, 3, 1]]]
        r["sc_def"] = [["other2.py", [3, 0, 3, 1]]]
        self.assertEqual(analyze.classify(r)[0], "scippy_wrong")

    def test_class_cell(self):
        r = row(doc="sw_m.py", text="__class__", sc=[["local 3", [0, 0]]], sp=[["scip-python python p 0.0.0 `p.m`/C#g().", [0, 0]]],
                sc_def=[["sw_m.py", [9, 8, 9, 17]]], sp_def=[["sw_m.py", [8, 8, 8, 9]]])
        self.assertEqual(analyze.classify(r)[0], "sp_wrong")
        r["sc_def"] = [["sw_m.py", [0, 0, 0, 1]]]  # not an assignment
        self.assertEqual(analyze.classify(r)[0], "scippy_wrong")
        r["sc_def"] = [["sw_m.py", [9, 8, 9, 17]]]
        r["text"] = "other"
        self.assertEqual(analyze.classify(r)[0], "scippy_wrong")


class BuiltinTypeAttr(unittest.TestCase):
    def test_attribute_named_type(self):
        r = row(kind="sp_missing_external", cat="attribute", text="type", sp=[[SP + "builtins/type#", [0, 0]]],
                sc=[["scip-scippy python p 0.0.0 . m/C#type.", [0, 0]]])
        self.assertTrue(analyze.sp_builtin_type_attr(r))
        self.assertEqual(analyze.classify(r)[0], "sp_builtin_type_attr")

    def test_other_attribute_stays_in_the_generic_bucket(self):
        r = row(kind="sp_missing_external", cat="attribute", text="other", sp=[[SP + "builtins/type#", [0, 0]]])
        self.assertFalse(analyze.sp_builtin_type_attr(r))
        self.assertEqual(analyze.classify(r)[0], "scip_python_missing_or_wrong")

    def test_name_is_not_an_attribute(self):
        r = row(kind="sp_missing_external", cat="name", text="type", sp=[[SP + "builtins/type#", [0, 0]]])
        self.assertFalse(analyze.sp_builtin_type_attr(r))


class Metrics(unittest.TestCase):
    def make(self):
        d = tempfile.mkdtemp(prefix="oracle-cmp-")
        stats = {"sp_inproj": 100, "sc_inproj": 110, "both": 90, "match_overlap": 80, "match_strict": 78}
        write(os.path.join(d, "summary.json"), json.dumps({"stats": stats, "builtins": {}}))
        bk = {
            "both_plausible / sp_points_at_owner_class, Scippy at the member": 5,
            "both_plausible / import_prefix: x": 2,
            "both_plausible / import_prefix_at_usage_site: y": 1,
            "both_plausible / same_name_member_in_another_class (inheritance/override)": 1,
            "sp_first_binding / z": 1,
            "scippy_wrong / W3_member_other": 0,
            "sp_builtin_type_attr / t": 4,
            "scippy_missing / C_rebind: r": 6,
        }
        write(os.path.join(d, "buckets.json"), json.dumps({k: {"count": v, "samples": []} for k, v in bk.items()}))
        return d

    def test_named_numbers(self):
        n = metrics.compute(self.make())["named"]
        self.assertAlmostEqual(n["strict_recall_pct"], 78.0)
        self.assertAlmostEqual(n["strict_precision_pct"], 100.0 * 78 / 90, places=3)
        self.assertEqual((n["owner_class_coarse"], n["import_prefix"], n["same_name_other_class"]), (5, 3, 1))
        self.assertEqual((n["sp_first_binding"], n["real_wrong"], n["sp_builtin_type_attr"]), (1, 0, 4))
        # 90 both = 80 overlap match + 10 different. Five classes hold 10 positions, so no alias-only match.
        self.assertEqual(n["match_through_alias_follow"], 0)

    def test_render_names_the_numbers(self):
        text = metrics.render(metrics.compute(self.make()))
        for word in ("strict recall", "owner-class coarse", "import-prefix", "same-name", "real wrong", "legacy"):
            self.assertIn(word, text)


class MetricsMulti(unittest.TestCase):
    """A multi-symbol position is never exact. It is covered when the set holds the target."""

    def make(self, **extra):
        d = tempfile.mkdtemp(prefix="oracle-cmp-")
        stats = {"sp_inproj": 100, "sc_inproj": 110, "both": 90, "match_overlap": 80, "match_strict": 78,
                 "multi_symbol": 7, "both_multi": 6, "multi_covered": 4, "multi_not_covered": 2}
        stats.update(extra)
        write(os.path.join(d, "summary.json"), json.dumps({"stats": stats, "builtins": {}}))
        bk = {"multi_not_covered / sp_points_at_owner_class: x": 1, "multi_not_covered / no candidate has the scip-python target": 1}
        write(os.path.join(d, "buckets.json"), json.dumps({k: {"count": v, "samples": []} for k, v in bk.items()}))
        return d

    def test_exact_and_covered(self):
        n = metrics.compute(self.make())["named"]
        # Exact: 78 of 100 and 78 of 90. The six multi-symbol positions are not in `both`.
        self.assertAlmostEqual(n["strict_recall_pct"], 78.0)
        self.assertAlmostEqual(n["strict_precision_pct"], 100.0 * 78 / 90, places=3)
        self.assertAlmostEqual(n["covered_recall_pct"], 82.0)
        self.assertAlmostEqual(n["covered_precision_pct"], 100.0 * 82 / 96, places=3)
        self.assertEqual((n["multi_symbol"], n["multi_both"], n["multi_covered"], n["multi_not_covered"]), (7, 6, 4, 2))
        self.assertEqual(n["multi_owner_class_coarse"], 1)
        # The multi-symbol positions do not enter the alias-follow remainder of the exact numbers.
        self.assertEqual(n["match_through_alias_follow"], 10)

    def test_without_multi_symbol_positions_covered_equals_exact(self):
        d = self.make(multi_symbol=0, both_multi=0, multi_covered=0, multi_not_covered=0)
        n = metrics.compute(d)["named"]
        self.assertEqual(n["covered_recall_pct"], n["strict_recall_pct"])
        self.assertEqual(n["covered_precision_pct"], n["strict_precision_pct"])

    def test_render_names_the_multi_numbers(self):
        text = metrics.render(metrics.compute(self.make()))
        for word in ("covered recall", "multi-symbol positions", "exact"):
            self.assertIn(word, text)


class Track(unittest.TestCase):
    def test_add_and_delta(self):
        csv_path = os.path.join(tempfile.mkdtemp(), "c.csv")
        a, b = Metrics().make(), Metrics().make()
        bk = oraclelib.read_json(os.path.join(b, "buckets.json"))
        bk["scippy_wrong / W3_member_other"]["count"] = 3
        write(os.path.join(b, "buckets.json"), json.dumps(bk))
        for d in (a, b):
            write(os.path.join(d, "metrics.json"), json.dumps(metrics.compute(d)))
        with contextlib.redirect_stdout(io.StringIO()):
            track.add(csv_path, "base", {"mypy": a}, "2026-01-01")
            track.add(csv_path, "next", {"mypy": b}, "2026-01-02")
        with self.assertRaises(SystemExit):
            track.add(csv_path, "base", {"mypy": a})
        out = io.StringIO()
        track.delta(csv_path, "next", "base", out=out)
        text = out.getvalue()
        self.assertIn("scippy_wrong / W3_member_other", text)
        self.assertIn("real_wrong", text)
        self.assertIn("+3", text)
        self.assertNotIn("sp_builtin_type_attr / t", text)  # unchanged rows stay out


# A minimal SCIP writer for the gate test.
def varint(n):
    out = b""
    while True:
        b = n & 0x7F
        n >>= 7
        out += bytes([b | (0x80 if n else 0)])
        if not n:
            return out


def fld(no, wire, payload):
    tag = varint(no << 3 | wire)
    return tag + (varint(len(payload)) + payload if wire == 2 else payload)


def scip(tool, occs):
    """occs: [(line, col, end_col, symbol, roles)] in one document `m.py`."""
    doc = fld(1, 2, b"m.py")
    for line, c0, c1, sym, roles in occs:
        o = fld(1, 2, b"".join(varint(x) for x in (line, c0, c1))) + fld(2, 2, sym.encode()) + fld(3, 0, varint(roles))
        doc += fld(2, 2, o)
    meta = fld(2, 2, fld(1, 2, tool.encode()) + fld(2, 2, b"1"))
    return fld(1, 2, meta) + fld(2, 2, doc)


SRC = "def f(): pass\nf()\nf()\n"
DEF, REF = 1, 0
SPF = SP + "m/f()."
SCF = "scip-scippy python p 0.0.0 . m/f()."


class Gate(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        write(os.path.join(oraclelib.ORACLE_DIR, "corpora", "t", "root", "m.py"), SRC)
        ref = scip("scip-python", [(0, 4, 5, SPF, DEF), (1, 0, 1, SPF, REF), (2, 0, 1, SPF, REF)])
        os.makedirs(os.path.join(oraclelib.ORACLE_DIR, "ref"), exist_ok=True)
        with open(os.path.join(oraclelib.ORACLE_DIR, "ref", "t.scip"), "wb") as f:
            f.write(ref)
        cls.base = cls.run_dir("base", [(0, 4, 5, SCF, DEF), (1, 0, 1, SCF, REF), (2, 0, 1, SCF, REF)])
        os.makedirs(os.path.join(oraclelib.ORACLE_DIR, "baseline"), exist_ok=True)
        os.replace(cls.base, os.path.join(oraclelib.ORACLE_DIR, "baseline", "t"))
        cls.base = os.path.join(oraclelib.ORACLE_DIR, "baseline", "t")

    @staticmethod
    def run_dir(name, occs):
        d = os.path.join(_SCRATCH, "run-" + name)
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, "scippy.scip"), "wb") as f:
            f.write(scip("scippy", occs))
        write(os.path.join(d, "scippy.trace"), "")
        return d

    def gate(self, rundir, *extra, allow=None):
        allow_path = os.path.join(_SCRATCH, "allow-%d.txt" % len(os.listdir(_SCRATCH)))
        write(allow_path, allow or "")
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            status = gate.run(["t=" + rundir, "--allow", allow_path, "--zero-wrong", "t"] + list(extra))
        return status, out.getvalue()

    def test_same_run_passes(self):
        status, text = self.gate(self.run_dir("same", [(0, 4, 5, SCF, DEF), (1, 0, 1, SCF, REF), (2, 0, 1, SCF, REF)]))
        self.assertEqual(status, 0, text)

    def test_lost_location_fails_and_allowlist_passes(self):
        d = self.run_dir("lost", [(0, 4, 5, SCF, DEF), (1, 0, 1, SCF, REF)])
        status, text = self.gate(d, "--max-strict-drop", "100")
        self.assertEqual(status, 1, text)
        self.assertIn("m.py:3:0  resolved -> unresolved", text)
        status, text = self.gate(d, "--max-strict-drop", "100", allow="t\tm.py:3:0\tdeliberate\n")
        self.assertEqual(status, 0, text)

    def test_strict_drop_fails(self):
        d = self.run_dir("drop", [(0, 4, 5, SCF, DEF), (1, 0, 1, SCF, REF)])
        status, text = self.gate(d, allow="t\tm.py:3:0\tdeliberate\n")
        self.assertEqual(status, 1, text)
        self.assertIn("strict_recall_pct fell", text)

    def test_two_symbols_at_one_occurrence_fail(self):
        other = "scip-scippy python p 0.0.0 . m/g()."
        d = self.run_dir("multi", [(0, 4, 5, SCF, DEF), (1, 0, 1, SCF, REF), (1, 0, 1, other, REF), (2, 0, 1, SCF, REF)])
        status, text = self.gate(d, "--max-strict-drop", "100", allow="t\tm.py:2:0\tdeliberate\n")
        self.assertEqual(status, 1, text)
        self.assertIn("more than one symbol", text)

    def test_real_wrong_fails(self):
        g = "scip-scippy python p 0.0.0 . m/g()."
        d = self.run_dir("wrong", [(0, 4, 5, SCF, DEF), (2, 0, 1, SCF, REF), (1, 0, 1, g, REF), (0, 0, 1, g, DEF)])
        status, text = self.gate(d, "--max-strict-drop", "100", allow="t\tm.py:2:0\tdeliberate\n")
        self.assertEqual(status, 1, text)
        self.assertIn("real wrong is", text)

    def test_allowlist_needs_a_reason(self):
        p = os.path.join(_SCRATCH, "bad-allow.txt")
        write(p, "t\tm.py:3:0\n")
        with self.assertRaises(SystemExit):
            gate.read_allow(p)


SRC_U = (
    "class A:\n"                 # 0
    "    def go(self): pass\n"   # 1: go at columns 8 to 10
    "class B:\n"                 # 2
    "    def go(self): pass\n"   # 3
    "class C(A):\n"              # 4: A at column 8
    "    def go(self): pass\n"   # 5
    "def use(x):\n"              # 6
    "    x.go()\n"               # 7: go at column 6
    "    x.go()\n"               # 8
)
PY = "scip-python python p 0.0.0 "
SC = "scip-scippy python p 0.0.0 . "


def sc(name):
    return SC + "m/" + name


def u_defs(sym):
    """The definitions of the classes and methods of SRC_U, and the base reference."""
    return [
        (0, 6, 7, sym("A#"), DEF), (1, 8, 10, sym("A#go()."), DEF),
        (2, 6, 7, sym("B#"), DEF), (3, 8, 10, sym("B#go()."), DEF),
        (4, 6, 7, sym("C#"), DEF), (5, 8, 10, sym("C#go()."), DEF),
        (4, 8, 9, sym("A#"), REF),
    ]


def py(name):
    return PY + "m/" + name


class GateMulti(unittest.TestCase):
    """Several symbols at one range pass for a true union and fail for any other set."""

    @classmethod
    def setUpClass(cls):
        write(os.path.join(oraclelib.ORACLE_DIR, "corpora", "u", "root", "m.py"), SRC_U)
        ref = scip("scip-python", u_defs(py) + [(7, 6, 8, py("A#go()."), REF), (8, 6, 8, py("A#go()."), REF)])
        with open(os.path.join(oraclelib.ORACLE_DIR, "ref", "u.scip"), "wb") as f:
            f.write(ref)
        base = Gate.run_dir("ubase", u_defs(sc) + [(8, 6, 8, sc("A#go()."), REF)])
        os.replace(base, os.path.join(oraclelib.ORACLE_DIR, "baseline", "u"))

    def gate(self, name, at7):
        d = Gate.run_dir(name, u_defs(sc) + [(8, 6, 8, sc("A#go()."), REF)] + [(7, 6, 8, s, REF) for s in at7])
        allow = os.path.join(_SCRATCH, "allow-u-%s.txt" % name)
        write(allow, "")
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            status = gate.run(["u=" + d, "--allow", allow, "--zero-wrong", "u"])
        return status, out.getvalue(), d

    def test_a_union_of_unrelated_classes_passes_and_is_covered_not_exact(self):
        status, text, d = self.gate("u-union", [sc("A#go()."), sc("B#go().")])
        self.assertEqual(status, 0, text)
        self.assertIn("occurrences with more than one symbol: 1 (baseline 0); true unions 1, other sets 0", text)
        n = oraclelib.read_json(os.path.join(d, "gate-cmp", "metrics.json"))["named"]
        self.assertEqual((n["multi_symbol"], n["multi_covered"], n["multi_not_covered"]), (1, 1, 0))
        # The union position is covered, never exact.
        self.assertGreater(n["covered_recall_pct"], n["strict_recall_pct"])
        # The exact numbers do not move: the union position is not in them.
        self.assertIn("strict_recall_pct %s (baseline %s, change +0.0000 pp)" % ((("%.4f" % n["strict_recall_pct"]),) * 2), text)

    def test_a_union_that_misses_the_target_is_not_covered(self):
        status, text, d = self.gate("u-miss", [sc("B#go()."), sc("C#go().")])
        n = oraclelib.read_json(os.path.join(d, "gate-cmp", "metrics.json"))["named"]
        self.assertEqual((n["multi_covered"], n["multi_not_covered"]), (0, 1))

    def test_an_override_chain_passes(self):
        # C inherits from A (the base reference at line 4).
        status, text, _ = self.gate("u-related", [sc("A#go()."), sc("C#go().")])
        self.assertEqual(status, 0, text)
        self.assertIn("true unions 1, other sets 0", text)

    def test_related_owners_that_are_not_a_chain_from_one_base_fail(self):
        # C inherits from A, and B is not related to A.
        status, text, _ = self.gate("u-notchain", [sc("A#go()."), sc("C#go()."), sc("B#go().")])
        self.assertEqual(status, 1, text)
        self.assertIn("owners are related", text)

    def test_an_attribute_chain_fails(self):
        status, text, _ = self.gate("u-attr", [sc("A#x."), sc("C#x.")])
        self.assertEqual(status, 1, text)
        self.assertIn("owners are related", text)

    def test_the_same_owner_fails(self):
        status, text, _ = self.gate("u-same", [sc("A#go()."), sc("A#other().")])
        self.assertEqual(status, 1, text)
        self.assertIn("two symbols have one owner", text)

    def test_a_module_level_symbol_fails(self):
        status, text, _ = self.gate("u-module", [sc("A#go()."), sc("go().")])
        self.assertEqual(status, 1, text)
        self.assertIn("not a member of a class", text)


class Owners(unittest.TestCase):
    def test_owner_of(self):
        self.assertEqual(owners.owner_of(sc("A#go().")), sc("A#"))
        self.assertEqual(owners.owner_of(sc("A#x.")), sc("A#"))
        self.assertEqual(owners.owner_of(sc("Outer#Inner#x.")), sc("Outer#Inner#"))
        for sym in (sc("A#"), sc("f()."), sc("f().(a)"), "local 3", sc("v.")):
            self.assertIsNone(owners.owner_of(sym), sym)

    def test_related_follows_transitive_bases_and_ends_on_a_cycle(self):
        bases = {"C": {"B"}, "B": {"A"}, "X": {"Y"}, "Y": {"X"}}
        self.assertTrue(owners.related("C", "A", bases))
        self.assertTrue(owners.related("A", "C", bases))
        self.assertTrue(owners.related("A", "A", bases))
        self.assertFalse(owners.related("A", "D", bases))
        self.assertTrue(owners.related("X", "Y", bases))
        self.assertFalse(owners.related("X", "A", bases))

    def test_the_owners_of_a_set(self):
        bases = {sc("C#"): {sc("B#")}, sc("B#"): {sc("A#")}}
        go = lambda *cs: [sc(c + "#go().") for c in cs]  # noqa: E731
        # Unrelated owners pass, and the order does not matter.
        self.assertIsNone(owners.set_problem(go("A", "D"), bases))
        # An override chain from one base passes, with a skipped level too.
        self.assertIsNone(owners.set_problem(go("A", "B", "C"), bases))
        self.assertIsNone(owners.set_problem(go("C", "A"), bases))
        self.assertIsNone(owners.set_problem(go("A", "C"), bases))
        # Related owners with an unrelated third owner are not a chain from one base.
        self.assertIn("owners are related", owners.set_problem(go("A", "C", "D"), bases))
        # Two siblings of one base without the base are related to no one: they pass.
        bases2 = {sc("B#"): {sc("A#")}, sc("C#"): {sc("A#")}}
        self.assertIsNone(owners.set_problem(go("B", "C"), bases2))
        # A chain needs the base in the set: two siblings and a subclass of one are not a chain.
        bases3 = dict(bases2, **{sc("E#"): {sc("B#")}})
        self.assertIn("owners are related", owners.set_problem(go("B", "C", "E"), bases3))
        # An attribute chain has one target.
        self.assertIn("owners are related", owners.set_problem([sc("A#x."), sc("C#x.")], bases))
        # Methods with different names are not a chain.
        self.assertIn("owners are related", owners.set_problem([sc("A#go()."), sc("C#run().")], bases))
        # The same owner fails.
        self.assertIn("one owner", owners.set_problem([sc("A#go()."), sc("A#x.")], bases))

    def test_member_name(self):
        self.assertEqual(owners.member_name(sc("A#go().")), "go")
        self.assertIsNone(owners.member_name(sc("A#x.")))

    def test_cap(self):
        syms = [sc("C%d#go()." % i) for i in range(5)]
        self.assertIn("5 symbols", owners.set_problem(syms, {}))
        self.assertIsNone(owners.set_problem(syms[:4], {}))


class Diff(unittest.TestCase):
    def test_diff(self):
        a = ("m.py", (1, 0))
        b = ("m.py", (2, 0))
        c = ("m.py", (3, 0))
        base = {a: frozenset({("ext", "x")}), b: frozenset({("ext", "y")})}
        cand = {a: frozenset({("ext", "z")}), c: frozenset({("ext", "y")})}
        d = difftool.diff(base, cand)
        self.assertEqual((d["changed"], d["lost"], d["gained"]), ([a], [b], [c]))
        self.assertEqual(difftool.fmt(a), "m.py:2:0")


if __name__ == "__main__":
    unittest.main()
