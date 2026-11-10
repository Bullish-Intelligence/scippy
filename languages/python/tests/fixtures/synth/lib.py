from dataclasses import dataclass
from typing import NamedTuple


@dataclass
class Point:
    x: int
    y: int = 0


class Sig(NamedTuple):
    name: str
    args: list


def wrap(*args, **kwargs):
    pass
