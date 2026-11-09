class Box:
    def __init__(self, size, label="x"):
        self.size = size
        self.label = label

    def resize(self, size, *, force=False):
        size = max(size, 1)
        return size

    @classmethod
    def unit(cls, label):
        return cls(label)


def make(width, height=1):
    return Box(width)


def outer(a):
    def inner(b):
        return b

    return inner(a)
