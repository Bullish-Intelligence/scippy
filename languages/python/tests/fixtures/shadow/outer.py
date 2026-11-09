def process(value):
#   ^^^^^^^ definition scippy python workspace . outer/process().
#           ^^^^^ definition scippy python workspace . outer/process().(value)
    return value
#          ^^^^^ reference scippy python workspace . outer/process().(value)


def local_shadow(process):
#   ^^^^^^^^^^^^ definition scippy python workspace . outer/local_shadow().
#                ^^^^^^^ definition scippy python workspace . outer/local_shadow().(process)
    return process
#          ^^^^^^^ reference scippy python workspace . outer/local_shadow().(process)


def rebinding():
#   ^^^^^^^^^ definition scippy python workspace . outer/rebinding().
    process = 1
#   ^^^^^^^ definition local 0
    return process
#          ^^^^^^^ reference local 0


def calls_outer():
#   ^^^^^^^^^^^ definition scippy python workspace . outer/calls_outer().
    return process(2)
#          ^^^^^^^ reference scippy python workspace . outer/process().
