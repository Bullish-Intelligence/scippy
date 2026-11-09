class Failure(ValueError):
    pass


def measure(items):
    n = len(items)
    ok = isinstance(n, int)
    label = str(n)
    return str.join(",", [label]), ok
