from demo.a import alpha_total, Widget


def use(seed):
    widget = Widget()
    widget.run(seed)
    first = alpha_total(seed, seed + 1)
    second = alpha_total(first, seed)
    return first + second
