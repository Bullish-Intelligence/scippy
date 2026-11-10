from dataclasses import dataclass
from enum import Enum


class Color(Enum):
    RED = 1


@dataclass
class Point:
    x: int
    y: int


def helper(value):
    return value


def handle(event, color):
    match helper(event):
        case Point(x=0, y=yy) if yy > 0:
            return helper(yy)
        case Color.RED:
            return color
        case [first, *rest]:
            return helper(first), rest
        case {"k": val, **extra}:
            return val, extra
        case Point() as shape:
            return shape
        case captured:
            return captured
        case _:
            return helper(event)


def leak(event):
    match event:
        case [left, right]:
            pass
    return left, right
