def shared(a, b):
    ordered = sorted([a, b])
    low = ordered[0]
    high = ordered[1]
    if low == high:
        return [low]
    return [low, high]
