def process(value):
#   ^^^^^^^ definition scippy python workspace . outer/process().
#           ^^^^^ definition local 0
    return value
#          ^^^^^ reference local 0


def local_shadow(process):
#   ^^^^^^^^^^^^ definition scippy python workspace . outer/local_shadow().
#                ^^^^^^^ definition local 1
    return process
#          ^^^^^^^ reference local 1


def rebinding():
#   ^^^^^^^^^ definition scippy python workspace . outer/rebinding().
    process = 1
#   ^^^^^^^ definition local 2
    return process
#          ^^^^^^^ reference local 2


def calls_outer():
#   ^^^^^^^^^^^ definition scippy python workspace . outer/calls_outer().
    return process(2)
#          ^^^^^^^ reference scippy python workspace . outer/process().
