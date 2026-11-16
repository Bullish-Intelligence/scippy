REGISTRY = {"alpha": 1, "beta": 2}


def alpha(x, y):
    total = x + y
    scaled = total * 2
    if scaled > 10:
        scaled = scaled - y
    return scaled + x


def beta(left, right):
    pairs = [(left, right), (right, left)]
    best = max(pairs)
    return best[0] - best[1]
