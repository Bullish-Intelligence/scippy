from .a import from_a
#              ^^^^^^ reference scippy python workspace . lib/pkg/a/from_a().


def from_b():
#   ^^^^^^ definition scippy python workspace . lib/pkg/b/from_b().
    return from_a()
#          ^^^^^^ reference scippy python workspace . lib/pkg/a/from_a().
