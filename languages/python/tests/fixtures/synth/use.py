from lib import Point, Sig, wrap

count = 0

Point(x=1, y=2)
Sig(name="f", args=[])
wrap(args=1)


def bump():
    global count
    count = count + 1
    return count
