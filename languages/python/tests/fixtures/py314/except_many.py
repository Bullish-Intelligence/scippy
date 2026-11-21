from base import Boom, Bust, Crash, Fail


def run():
    try:
        pass
    except Boom, Bust, Crash:
        return 1
    except Fail, Bust:
        return 2
    return 0
