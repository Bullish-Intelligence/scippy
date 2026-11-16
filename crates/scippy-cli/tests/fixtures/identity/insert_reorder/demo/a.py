def gamma(first, second, third):
    mixed = first * second + third
    window = [first, second, third]
    window.sort()
    return mixed - window[1]


def beta(left, right):
    pairs = [(left, right), (right, left)]
    best = max(pairs)
    return best[0] - best[1]


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


REGISTRY = {"alpha": 1, "beta": 2}


def alpha(x, y):
    total = x + y
    scaled = total * 2
    if scaled > 10:
        scaled = scaled - y
    return scaled + x
