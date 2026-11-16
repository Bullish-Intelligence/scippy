REGISTRY = {"alpha": 1, "beta": 2}


def alpha(x, y):
    total = x + y
    scaled = total * 2
    if scaled > 10:
        scaled = scaled - y
    return scaled + x


class Draft:
    def run(self, count):
        names = [str(n) for n in range(count)]
        names.sort(reverse=True)
        return ",".join(names)

    def reset(self, value):
        self.value = None
        self.log = ["reset", value]
        return len(self.log)


def beta(left, right):
    pairs = [(left, right), (right, left)]
    best = max(pairs)
    return best[0] - best[1]
