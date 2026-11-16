from demo.aa import alpha, Widget


def use(seed):
    widget = Widget()
    widget.run(seed)
    first = alpha(seed, seed + 1)
    second = alpha(first, seed)
    return first + second
