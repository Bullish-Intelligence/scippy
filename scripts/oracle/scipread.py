"""Minimal SCIP protobuf reader (no dependencies). Run with python3 -I."""
import sys


def _varint(b, i):
    r = 0
    s = 0
    while True:
        c = b[i]
        i += 1
        r |= (c & 0x7F) << s
        if c < 0x80:
            return r, i
        s += 7


def _fields(b):
    """Yield (field_no, wire_type, value). value is int or bytes."""
    i = 0
    n = len(b)
    while i < n:
        k, i = _varint(b, i)
        f, w = k >> 3, k & 7
        if w == 0:
            v, i = _varint(b, i)
        elif w == 2:
            ln, i = _varint(b, i)
            v = b[i:i + ln]
            i += ln
        elif w == 1:
            v = b[i:i + 8]
            i += 8
        elif w == 5:
            v = b[i:i + 4]
            i += 4
        else:
            raise ValueError("wire type %d" % w)
        yield f, w, v


def _packed(b):
    out = []
    i = 0
    while i < len(b):
        v, i = _varint(b, i)
        out.append(v)
    return out


def read_index(path):
    """Return dict: tool, documents: {path: {occs: [(range4, symbol, roles)], syms: {symbol: (kind, name)}}}."""
    data = open(path, "rb").read()
    tool = None
    docs = {}
    for f, w, v in _fields(data):
        if f == 1:
            for f2, _, v2 in _fields(v):
                if f2 == 2:
                    name = ver = ""
                    for f3, _, v3 in _fields(v2):
                        if f3 == 1:
                            name = v3.decode()
                        elif f3 == 2:
                            ver = v3.decode()
                    tool = (name, ver)
        elif f == 2:
            path_ = ""
            enc = 0
            occs = []
            syms = {}
            for f2, w2, v2 in _fields(v):
                if f2 == 1:
                    path_ = v2.decode()
                elif f2 == 2:
                    rng = []
                    sym = ""
                    roles = 0
                    for f3, w3, v3 in _fields(v2):
                        if f3 == 1:
                            rng = _packed(v3) if w3 == 2 else rng + [v3]
                        elif f3 in (8, 9):
                            m = {1: 0, 2: 0, 3: 0, 4: 0}
                            for f4, _, v4 in _fields(v3):
                                m[f4] = v4
                            if f3 == 8:
                                rng = [m[1], m[2], m[1], m[3]]
                            else:
                                rng = [m[1], m[2], m[3], m[4]]
                        elif f3 == 2:
                            sym = v3.decode()
                        elif f3 == 3:
                            roles = v3
                    if len(rng) == 3:
                        rng = [rng[0], rng[1], rng[0], rng[2]]
                    occs.append((tuple(rng), sym, roles))
                elif f2 == 6:
                    enc = v2
                elif f2 == 3:
                    s = ""
                    kind = 0
                    name = ""
                    for f3, _, v3 in _fields(v2):
                        if f3 == 1:
                            s = v3.decode()
                        elif f3 == 5:
                            kind = v3
                        elif f3 == 6:
                            name = v3.decode()
                    syms[s] = (kind, name)
            docs[path_] = {"occs": occs, "syms": syms, "enc": enc}
    return {"tool": tool, "documents": docs}


if __name__ == "__main__":
    ix = read_index(sys.argv[1])
    print(ix["tool"], len(ix["documents"]), sum(len(d["occs"]) for d in ix["documents"].values()))
