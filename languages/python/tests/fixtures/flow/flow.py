import sys


def make():
    return 1


class Base:
    def __init__(self):
        self.a = make()
#       ^^^^ reference scippy python workspace . flow/Base#__init__().(self)
#                ^^^^ reference scippy python workspace . flow/make().


class Child(Base):
    def __init__(self, v):
        super().__init__()
        self.v = v
#       ^^^^ reference scippy python workspace . flow/Child#__init__().(self)
#                ^ reference scippy python workspace . flow/Child#__init__().(v)
        self.w = self.v
#       ^^^^ reference scippy python workspace . flow/Child#__init__().(self)
#                ^^^^ reference scippy python workspace . flow/Child#__init__().(self)
#                     ^ reference scippy python workspace . flow/Child#v.

    def go(self):
        return self.w
#              ^^^^ reference scippy python workspace . flow/Child#go().(self)
#                   ^ reference scippy python workspace . flow/Child#w.


def run():
    x = make()
#   ^ definition local 0
    return x
#          ^ reference local 0


if sys.argv:
    from lib import f
else:
    def f():
        return 2

value = f()
