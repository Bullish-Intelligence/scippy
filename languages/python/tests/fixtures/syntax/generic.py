class BaseModel:
    pass


T = 1


class Box[T: BaseModel]:
    item: T

    def get(self) -> T:
        return self.item


def first[T](items: list[T]) -> T:
    return items[0]


type Pair[T] = tuple[T, T]
type Alias = int


def use(a: Alias, p: Pair, n: T) -> None:
    pass
