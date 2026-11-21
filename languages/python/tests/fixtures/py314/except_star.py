from base import Boom, Bust, Crash


def run():
    try:
        pass
    except* Boom, Bust:
        return 1
    except* Crash:
        return 2
    return 0
