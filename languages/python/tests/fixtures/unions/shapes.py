class ListExpr:
    def __init__(self):
        self.items = []

    def render(self):
        return "list"


class TupleExpr:
    def __init__(self):
        self.items = ()

    def render(self):
        return "tuple"


class Base:
    def render(self):
        return "base"


class Child(Base):
    def render(self):
        return "child"


class Grand(Child):
    pass


class Solo:
    def render(self):
        return "solo"


class One:
    def size(self):
        return 1


class Two:
    def size(self):
        return 2


class Three:
    def size(self):
        return 3


class Four:
    def size(self):
        return 4


class Five:
    def size(self):
        return 5


def union_annotation(e: ListExpr | TupleExpr):
    return e.items


def tuple_guard(x):
    if isinstance(x, (ListExpr, TupleExpr)):
        return x.items
    return None


def method_union(u: ListExpr | TupleExpr):
    return u.render()


def related_owners(r: Base | Child):
    return r.render()


def related_through_a_middle_class(g: Base | Grand):
    return g.render()


def unrelated_with_a_related_pair(m: Solo | Base | Child):
    return m.render()


def four_classes(f: One | Two | Three | Four):
    return f.size()


def five_classes(v: One | Two | Three | Four | Five):
    return v.size()
