total = 0


def count(items):
    if (found := len(items)) > 1:
        return found
    kept = [y for x in items if (y := x + total) > 0]
    return kept, y
