from lib import Child, Grandchild, Item, Model, MoreOpts, NotModel, Opts, Point, Sig, wrap

count = 0

Point(x=1, y=2)
Sig(name="f", args=[])
wrap(args=1)
Model(title="t")
Grandchild(title="t", extra=1, deep=True)
Item[int](value=1)
Opts(quiet=True)
MoreOpts(quiet=True, level=1)
NotModel(own=1)


def bump():
    global count
    count = count + 1
    return count
