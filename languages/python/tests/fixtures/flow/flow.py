import sys


def make():
    return 1


class Base:
    def __init__(self):
        self.a = make()
#       ^^^^ reference local 0
#                ^^^^ reference scippy python workspace . flow/make().


class Child(Base):
    def __init__(self, v):
        super().__init__()
        self.v = v
#       ^^^^ reference local 1
#                ^ reference local 2
        self.w = self.v
#       ^^^^ reference local 1
#                ^^^^ reference local 1
#                     ^ reference scippy python workspace . flow/Child#v.

    def go(self):
        return self.w
#              ^^^^ reference local 3
#                   ^ reference scippy python workspace . flow/Child#w.


def run():
    x = make()
#   ^ definition local 4
    return x
#          ^ reference local 4


if sys.argv:
    from lib import f
else:
    def f():
        return 2

value = f()
