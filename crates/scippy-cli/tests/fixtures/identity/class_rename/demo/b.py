from demo.a import alpha, Gadget


def use(seed):
    widget = Gadget()
    widget.run(seed)
    first = alpha(seed, seed + 1)
    second = alpha(first, seed)
    return first + second
