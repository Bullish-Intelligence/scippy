class Address:
    @classmethod
    def parse(cls, data) -> Address:
        return cls._f1(data)

    @classmethod
    def _f1(cls, data) -> Address:
        return cls._f2(data)

    @classmethod
    def _f2(cls, data) -> Address:
        return cls._f3(data)

    @classmethod
    def _f3(cls, data) -> Address:
        return cls._f4(data)

    @classmethod
    def _f4(cls, data) -> Address:
        return cls._f5(data)

    @classmethod
    def _f5(cls, data) -> Address:
        address = object.__new__(cls)
        return address

    def resolve(self, document) -> Resolution:
        return Resolution._make(self, document)


class Resolution:
    node: object

    @classmethod
    def _make(cls, address, document) -> Resolution:
        resolution = object.__new__(cls)
        return resolution
