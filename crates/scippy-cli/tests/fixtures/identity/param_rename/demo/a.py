REGISTRY = {"alpha": 1, "beta": 2}


def alpha(x, w):
    total = x + w
    scaled = total * 2
    if scaled > 10:
        scaled = scaled - w
    return scaled + x


class Widget:
    def run(self, count):
        items = []
        for index in range(count):
            items.append(index * count)
        return items

    def reset(self, value):
        self.value = value
        self.history = [value, value + 1]
        return self.history


def beta(left, right):
    pairs = [(left, right), (right, left)]
    best = max(pairs)
    return best[0] - best[1]
