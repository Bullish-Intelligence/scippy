"""Owner classes of a Scippy index, for the multi-symbol check of the gate.

Scippy can write several occurrences at one range, one for each member of a union (see
CHANGELOG.md). A set is a union when every symbol is a member of a class, there are at most
`MAX_SYMBOLS` symbols, and the owner classes are distinct. The owners can be related by
inheritance: the members of an override chain are a union.
"""
import re

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


def set_problem(symbols, cap=MAX_SYMBOLS):
    """Return the reason that a multi-symbol set is not a union, or None.

    `symbols` are the symbols at one range. The owners can be related by inheritance: a base
    declaration and its overrides are an override chain. No two symbols can have one owner.
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
    return None
