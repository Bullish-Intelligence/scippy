class Boom(Exception):
    pass


class Bust(Exception):
    pass


class Crash(Exception):
    pass


class Fail(Exception):
    pass


def render(value):
    return str(value)
