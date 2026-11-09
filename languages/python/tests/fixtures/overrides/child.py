import base
from base import Base as Root, Left, Right


class Child(Root):
    def run(self):
        return 10

    def extra(self):
        return 11


class Grand(Child):
    def run(self):
        return 20

    def stop(self):
        return 21


class Qualified(base.Base):
    def keep(self):
        return 30


class Several(Left, Right):
    def spin(self):
        return 40


class Unrelated:
    def run(self):
        return 50


class Failure(Exception):
    def __str__(self):
        return "failure"


class Missing(NotDefined):
    def run(self):
        return 60
