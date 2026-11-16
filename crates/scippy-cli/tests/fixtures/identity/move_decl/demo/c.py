def alpha(x, y):
    total = x + y
    scaled = total * 2
    if scaled > 10:
        scaled = scaled - y
    return scaled + x
