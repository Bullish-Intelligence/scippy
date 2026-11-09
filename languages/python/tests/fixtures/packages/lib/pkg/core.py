def helper():
#   ^^^^^^ definition scippy python workspace . lib/pkg/core/helper().
    return 1


class Box:
#     ^^^ definition scippy python workspace . lib/pkg/core/Box#
    def __init__(self):
#       ^^^^^^^^ definition scippy python workspace . lib/pkg/core/Box#__init__().
        self.value = 2
#            ^^^^^ definition scippy python workspace . lib/pkg/core/Box#value.

    def get(self):
#       ^^^ definition scippy python workspace . lib/pkg/core/Box#get().
        return self.value
#                   ^^^^^ reference scippy python workspace . lib/pkg/core/Box#value.
