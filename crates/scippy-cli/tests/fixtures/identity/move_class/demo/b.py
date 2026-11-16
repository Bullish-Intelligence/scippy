from demo.a import alpha
from demo.c import Widget


def use(seed):
    widget = Widget()
    widget.run(seed)
    first = alpha(seed, seed + 1)
    second = alpha(first, seed)
    return first + second
