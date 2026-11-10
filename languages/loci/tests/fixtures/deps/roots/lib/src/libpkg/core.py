from base.types import Base


class Engine(Base):
    def start(self):
        return self


def build():
    return Engine()
