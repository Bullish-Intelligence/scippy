"""Owner classes and class inheritance of a Scippy index, for the multi-symbol check of the gate.

Scippy can write several occurrences at one range, one for each member of a true union (see
CHANGELOG.md). A set is a true union when every symbol is a member of a class, there are at most
`MAX_SYMBOLS` symbols, no two symbols have one owner, and one of two cases holds:
  - no owner class is a subclass or a superclass of another owner; or
  - the set is an override chain of a base-typed receiver: all symbols are methods with one name,
    one owner is a common base of all other owners, and every other owner descends from it.
An attribute chain is not a union: an attribute chain has one target.

The inheritance comes from the sources of the corpus, not from the output of the resolver:
the scorer parses each class statement, and reads the symbol that an index gives to each name in
the list of bases. It reads the Scippy index and the scip-python index, and it joins both through
the position of the definition. A base that neither index resolves is unknown, and the scorer
cannot tell the relation.
"""
import ast
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import compare  # noqa: E402

MAX_SYMBOLS = 4

# The last descriptor of a symbol: a method `name().`, a term `name.`, or a type `name#`.
_LAST = re.compile(r"(?:`[^`]+`|[A-Za-z_0-9]+)(?:\(\)\.|\.|#)$")


def owner_of(sym):
    """Return the symbol of the class that owns a method or an attribute symbol, or None.

    `scippy python p . m/C#f().` and `scippy python p . m/C#x.` have the owner `...m/C#`.
    A class, a module-level name, a local symbol, and a parameter have no owner.
    """
    if sym.startswith("local ") or not (sym.endswith(".")):
        return None
    m = _LAST.search(sym)
    if not m:
        return None
    rest = sym[: m.start()]
    if not rest.endswith("#"):
        return None
    return rest


def _tail_position(expr):
    """The (line0, byte column) of the last name token of a base expression, or None."""
    if isinstance(expr, ast.Name):
        return expr.lineno - 1, expr.col_offset
    if isinstance(expr, ast.Attribute):
        if expr.end_lineno is None or expr.end_col_offset is None:
            return None
        return expr.end_lineno - 1, expr.end_col_offset - len(expr.attr)
    if isinstance(expr, ast.Subscript):
        return _tail_position(expr.value)
    return None


def class_bases(index_path, root, scippy_defs=None):
    """Return {class symbol: set of base class symbols} of one index.

    With `scippy_defs` (a map `(doc, (line, col))` -> Scippy class symbol), the symbols are those
    of Scippy: the tool maps each class through the position of its definition.
    """
    _, defs, refs, defpos = compare.load(index_path, root)
    cache = {}

    def parsed(doc):
        if doc not in cache:
            try:
                text = open(os.path.join(root, doc), encoding="utf-8", errors="replace").read()
                cache[doc] = (ast.parse(text), text.split("\n"))
            except (OSError, SyntaxError, ValueError):
                cache[doc] = None
        return cache[doc]

    def name_of(key):
        if scippy_defs is None:
            return key if isinstance(key, str) else None
        sites = defs.get(key, [])
        for doc, rng in sites:
            hit = scippy_defs.get((doc, tuple(rng[:2])))
            if hit:
                return hit
        return None

    out = {}
    for sym, sites in defs.items():
        if not isinstance(sym, str) or not sym.endswith("#"):
            continue
        me = name_of(sym)
        if me is None:
            continue
        for doc, rng in sites:
            tree = parsed(doc)
            if tree is None:
                continue
            ast_tree, lines = tree
            for node in ast.walk(ast_tree):
                if not isinstance(node, ast.ClassDef) or node.lineno - 1 != rng[0]:
                    continue
                for base in node.bases:
                    pos = _tail_position(base)
                    if pos is None or pos[0] >= len(lines):
                        continue
                    col = compare.to_cp(lines[pos[0]], pos[1], 1)
                    for _, key, roles in refs.get((doc, (pos[0], col)), []):
                        if roles & compare.DEF or not isinstance(key, str) or not key.endswith("#"):
                            continue
                        b = name_of(key)
                        if b:
                            out.setdefault(me, set()).add(b)
    return out


def load_bases(scippy_path, reference_path, root):
    """Return {class symbol of Scippy: set of base symbols of Scippy} from both indexes."""
    _, defs, _, _ = compare.load(scippy_path, root)
    at = {}
    for sym, sites in defs.items():
        if isinstance(sym, str) and sym.endswith("#"):
            for doc, rng in sites:
                at[(doc, tuple(rng[:2]))] = sym
    bases = class_bases(scippy_path, root)
    if reference_path:
        for cls, bs in class_bases(reference_path, root, at).items():
            bases.setdefault(cls, set()).update(bs)
    return bases


def ancestors(cls, bases):
    """Every direct or indirect base of a class. A cycle ends."""
    seen, todo = set(), list(bases.get(cls, ()))
    while todo:
        c = todo.pop()
        if c not in seen:
            seen.add(c)
            todo.extend(bases.get(c, ()))
    return seen


def related(a, b, bases):
    """True when two classes are one class, or one inherits from the other."""
    return a == b or b in ancestors(a, bases) or a in ancestors(b, bases)


def member_name(sym):
    """Return the name of a method symbol (`...C#go().` gives `go`), or None for any other symbol."""
    if not sym.endswith("()."):
        return None
    m = _LAST.search(sym)
    return m.group(0)[: -len("().")] if m else None


def _override_chain(symbols, owners, bases):
    """True when the symbols are one method name and one owner is a base of every other owner."""
    names = {member_name(s) for s in symbols}
    if None in names or len(names) != 1:
        return False
    return any(all(o == top or top in ancestors(o, bases) for o in owners) for top in owners)


def set_problem(symbols, bases, cap=MAX_SYMBOLS):
    """Return the reason that a multi-symbol set is not a true union, or None.

    `symbols` are the symbols at one range. `bases` is the map of `load_bases`.
    """
    if len(symbols) > cap:
        return "%d symbols (the cap is %d)" % (len(symbols), cap)
    owners = []
    for s in symbols:
        o = owner_of(s)
        if o is None:
            return "not a member of a class: %s" % s
        if o in owners:
            return "two symbols have one owner: %s" % o
        owners.append(o)
    pair = next(((a, b) for i, a in enumerate(owners) for b in owners[i + 1:] if related(a, b, bases)), None)
    if pair is None or _override_chain(symbols, owners, bases):
        return None
    return "owners are related and the set is not an override chain from one base: %s and %s" % pair
