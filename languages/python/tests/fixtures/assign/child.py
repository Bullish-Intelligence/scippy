from base import Base


class Child(Base):
    def __init__(self):
        super().__init__()
        self.size = 3
        self.extra = 4


def grow(item: Child, other: Base):
    item.size = 5
    item.extra = 6
    other.size = 7
    item.fresh = 8
