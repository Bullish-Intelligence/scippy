from base import Boom, Bust


class Box[T = Boom]:
    item: T

    def get(self) -> T:
        return self.item


def first[T: Bust = Boom, *Ts = *tuple[int], **P = [int]](items: list[T]) -> T:
    return items[0]


type Pair[T = Boom] = tuple[T, T]
