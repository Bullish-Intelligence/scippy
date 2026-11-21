from pkg import Address


def inner() -> Address:
    return Address.parse({})


def wrap() -> Address:
    return inner()


def use0(d):
    return wrap().resolve(d).node


def use1(d):
    return wrap().resolve(d).node


def use2(d):
    return wrap().resolve(d).node


def use3(d):
    return wrap().resolve(d).node


def use4(d):
    return wrap().resolve(d).node


def use5(d):
    return wrap().resolve(d).node


def use6(d):
    return wrap().resolve(d).node


def use7(d):
    return wrap().resolve(d).node


def use8(d):
    return wrap().resolve(d).node


def use9(d):
    return wrap().resolve(d).node


def use10(d):
    return wrap().resolve(d).node


def use11(d):
    return wrap().resolve(d).node


def use12(d):
    return wrap().resolve(d).node


def use13(d):
    return wrap().resolve(d).node


def use14(d):
    return wrap().resolve(d).node


def use15(d):
    return wrap().resolve(d).node


def use16(d):
    return wrap().resolve(d).node


def use17(d):
    return wrap().resolve(d).node


def use18(d):
    return wrap().resolve(d).node


def use19(d):
    return wrap().resolve(d).node
