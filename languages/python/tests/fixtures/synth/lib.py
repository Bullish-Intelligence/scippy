from dataclasses import dataclass
from typing import Generic, NamedTuple, TypedDict, TypeVar

from pydantic import BaseModel

T = TypeVar("T")


@dataclass
class Point:
    x: int
    y: int = 0


class Sig(NamedTuple):
    name: str
    args: list


def wrap(*args, **kwargs):
    pass


class Model(BaseModel):
    title: str
    _hidden: int = 0


class Child(Model):
    extra: int = 0


class Grandchild(Child):
    deep: bool = False


class Item(BaseModel, Generic[T]):
    value: T


class Opts(TypedDict, total=False):
    quiet: bool


class MoreOpts(Opts):
    level: int


class Plain:
    other: int = 0


class NotModel(Plain):
    own: int = 0
