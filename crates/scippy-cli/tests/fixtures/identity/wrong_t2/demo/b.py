from demo.a import alpha, Draft


def use(seed):
    widget = Draft()
    widget.run(seed)
    first = alpha(seed, seed + 1)
    second = alpha(first, seed)
    return first + second
