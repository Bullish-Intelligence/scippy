def process(value):
#   ^^^^^^^ definition scippy python workspace . other/process().
    return value


def unrelated():
#   ^^^^^^^^^ definition scippy python workspace . other/unrelated().
    return process(3)
#          ^^^^^^^ reference scippy python workspace . other/process().
