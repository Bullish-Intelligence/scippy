from demo.a import alpha


def use(seed):
    first = alpha(seed, seed + 1)
    second = alpha(first, seed)
    return first + second
