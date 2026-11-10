"""Source-context tags for a reference position. Uses the standard library only.

Parses the file with `ast` and `tokenize`. It never runs the code.
"""
import ast
import io
import os
import tokenize


class FileInfo:
    def __init__(self, root, doc):
        self.lines = []
        self.funcs = []  # (start_line0, end_line0, decorators[str], is_method)
        self.ann = []  # (l0, c0, l1, c1) annotation ranges (character columns)
        self.strs = []  # string constant ranges inside annotations
        self.kwarg = set()  # (line0, col) of identifier tokens that look like `name=` inside ( )
        self.deco_lines = set()
        self.import_lines = set()
        self.star_import_files = False
        self.store_names = set()
        self.store_attrs = set()
        self.aug = set()
        self.origins = {}  # func start line0 -> {name: origin}
        self.fnodes = {}
        try:
            src = open(os.path.join(root, doc), encoding="utf-8", errors="replace").read()
        except OSError:
            return
        self.lines = src.split("\n")
        self._tokens(src)
        try:
            tree = ast.parse(src)
        except SyntaxError:
            return
        self._walk(tree, False)
        for n in ast.walk(tree):
            if isinstance(n, ast.Name) and isinstance(n.ctx, (ast.Store, ast.Del)):
                self.store_names.add((n.lineno - 1, n.col_offset))
            elif isinstance(n, ast.Attribute) and isinstance(n.ctx, (ast.Store, ast.Del)) and n.end_lineno == n.lineno:
                self.store_attrs.add((n.end_lineno - 1, n.end_col_offset - len(n.attr)))
            elif isinstance(n, ast.AugAssign):
                t = n.target
                if isinstance(t, ast.Name):
                    self.aug.add((t.lineno - 1, t.col_offset))
                elif isinstance(t, ast.Attribute) and t.end_lineno == t.lineno:
                    self.aug.add((t.end_lineno - 1, t.end_col_offset - len(t.attr)))

    def _tokens(self, src):
        stack = []
        toks = []
        try:
            for t in tokenize.generate_tokens(io.StringIO(src).readline):
                toks.append(t)
        except (tokenize.TokenError, IndentationError, SyntaxError):
            pass
        for i, t in enumerate(toks):
            if t.type == tokenize.OP and t.string in "([{":
                stack.append(t.string)
            elif t.type == tokenize.OP and t.string in ")]}":
                if stack:
                    stack.pop()
            elif t.type == tokenize.NAME and stack and stack[-1] == "(":
                # next significant token
                j = i + 1
                while j < len(toks) and toks[j].type in (tokenize.NL, tokenize.COMMENT):
                    j += 1
                if j < len(toks) and toks[j].type == tokenize.OP and toks[j].string == "=":
                    self.kwarg.add((t.start[0] - 1, t.start[1]))

    def _walk(self, node, in_class):
        for ch in ast.iter_child_nodes(node):
            if isinstance(ch, (ast.FunctionDef, ast.AsyncFunctionDef)):
                decos = []
                for d in ch.decorator_list:
                    dd = d.func if isinstance(d, ast.Call) else d
                    decos.append(ast.unparse(dd))
                    self.deco_lines.add(d.lineno - 1)
                end = ch.end_lineno - 1
                self.funcs.append((ch.lineno - 1, end, decos, in_class))
                self.origins[ch.lineno - 1] = self._origins(ch)
                args = ch.args
                for a in args.posonlyargs + args.args + args.kwonlyargs + [args.vararg, args.kwarg]:
                    if a is not None and a.annotation is not None:
                        self._ann(a.annotation)
                if ch.returns is not None:
                    self._ann(ch.returns)
                self._walk(ch, False)
            elif isinstance(ch, ast.ClassDef):
                self._walk(ch, True)
            else:
                if isinstance(ch, ast.AnnAssign):
                    self._ann(ch.annotation)
                if isinstance(ch, (ast.Import, ast.ImportFrom)):
                    for l in range(ch.lineno - 1, ch.end_lineno):
                        self.import_lines.add(l)
                self._walk(ch, in_class)

    @staticmethod
    def _ann_kind(a):
        if isinstance(a, ast.Constant) and isinstance(a.value, str):
            return "string_ann"
        if isinstance(a, (ast.Name, ast.Attribute)):
            return "class_ann"
        if isinstance(a, ast.Subscript):
            h = ast.unparse(a.value).split(".")[-1]
            return "optional_ann" if h in ("Optional", "Union") else "generic_ann"
        if isinstance(a, ast.BinOp):
            return "union_ann"
        return "other_ann"

    def _origins(self, fn):
        o = {}
        args = fn.args
        for a in args.posonlyargs + args.args + args.kwonlyargs:
            o[a.arg] = ("param_" + self._ann_kind(a.annotation) + "|" + ast.unparse(a.annotation)) if a.annotation is not None else "param_unannotated"
        for a in (args.vararg, args.kwarg):
            if a is not None:
                o[a.arg] = "star_param"
        stack = list(fn.body)
        while stack:
            n = stack.pop(0)
            if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef, ast.Lambda)):
                continue
            if isinstance(n, ast.Assign):
                for t in n.targets:
                    if isinstance(t, ast.Name) and t.id not in o:
                        v = n.value
                        o[t.id] = "assign_call" if isinstance(v, ast.Call) else "assign_subscript" if isinstance(v, ast.Subscript) else "assign_attr" if isinstance(v, ast.Attribute) else "assign_other"
                    elif isinstance(t, (ast.Tuple, ast.List)):
                        for e in ast.walk(t):
                            if isinstance(e, ast.Name) and e.id not in o:
                                o[e.id] = "tuple_unpack"
            elif isinstance(n, ast.AnnAssign) and isinstance(n.target, ast.Name) and n.target.id not in o:
                o[n.target.id] = "local_" + self._ann_kind(n.annotation)
            elif isinstance(n, (ast.For, ast.AsyncFor)):
                for e in ast.walk(n.target):
                    if isinstance(e, ast.Name) and e.id not in o:
                        o[e.id] = "for_target"
            elif isinstance(n, (ast.With, ast.AsyncWith)):
                for it in n.items:
                    if it.optional_vars is not None:
                        for e in ast.walk(it.optional_vars):
                            if isinstance(e, ast.Name) and e.id not in o:
                                o[e.id] = "with_target"
            elif isinstance(n, ast.ExceptHandler) and n.name and n.name not in o:
                o[n.name] = "except_target"
            elif isinstance(n, ast.NamedExpr) and n.target.id not in o:
                o[n.target.id] = "walrus"
            for e in ast.walk(n) if isinstance(n, (ast.ListComp, ast.SetComp, ast.DictComp, ast.GeneratorExp)) else []:
                pass
            for ch in ast.iter_child_nodes(n):
                if not isinstance(ch, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef, ast.Lambda)):
                    stack.append(ch)
        return o

    def receiver_name(self, line0, col):
        l = self.lines[line0] if line0 < len(self.lines) else ""
        j = col - 2
        k = j
        while k >= 0 and (l[k].isalnum() or l[k] == "_"):
            k -= 1
        return l[k + 1:j + 1]

    def receiver_origin(self, line0, col):
        name = self.receiver_name(line0, col)
        f = self.enclosing_func(line0)
        if f is None:
            return "module_level"
        o = self.origins.get(f[0], {})
        if name in o:
            return o[name]
        return "not_in_func_scope(global/closure/comprehension)"

    def _ann(self, a):
        self.ann.append((a.lineno - 1, a.col_offset, a.end_lineno - 1, a.end_col_offset))
        for n in ast.walk(a):
            if isinstance(n, ast.Constant) and isinstance(n.value, str):
                self.strs.append((n.lineno - 1, n.col_offset, n.end_lineno - 1, n.end_col_offset))

    def bcol(self, line0, col):
        l = self.lines[line0] if line0 < len(self.lines) else ""
        return len(l[:col].encode("utf-8")) if not l.isascii() else col

    def in_ranges(self, rs, line0, col):
        b = self.bcol(line0, col)
        for l0, c0, l1, c1 in rs:
            if (l0, c0) <= (line0, b) < (l1, c1):
                return True
        return False

    def enclosing_func(self, line0):
        best = None
        for f in self.funcs:
            if f[0] <= line0 <= f[1]:
                if best is None or f[0] >= best[0]:
                    best = f
        return best

    def receiver_form(self, line0, col):
        """For `recv.name` return the form of the receiver."""
        l = self.lines[line0] if line0 < len(self.lines) else ""
        i = col - 1
        if i < 0 or l[i] != ".":
            return "name"
        j = i - 1
        if j < 0:
            return "multiline"
        c = l[j]
        if c in ")]":
            return "call_or_subscript_result"
        k = j
        while k >= 0 and (l[k].isalnum() or l[k] == "_"):
            k -= 1
        word = l[k + 1:j + 1]
        if k >= 0 and l[k] == ".":
            # chain a.b.name
            k2 = k - 1
            while k2 >= 0 and (l[k2].isalnum() or l[k2] == "_"):
                k2 -= 1
            first = l[k2 + 1:k]
            if k2 >= 0 and l[k2] == ".":
                return "chain3+"
            if first == "self":
                return "self.attr.name"
            return "local.attr.name"
        if word == "self":
            return "self.name"
        if word == "cls":
            return "cls.name"
        return "local.name"


_cache = {}


def info(root, doc):
    if doc not in _cache:
        _cache[doc] = FileInfo(root, doc)
    return _cache[doc]
