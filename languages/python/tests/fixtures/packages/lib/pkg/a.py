from .b import from_b
#              ^^^^^^ reference scippy python workspace . lib/pkg/b/from_b().


def from_a():
#   ^^^^^^ definition scippy python workspace . lib/pkg/a/from_a().
    return from_b()
#          ^^^^^^ reference scippy python workspace . lib/pkg/b/from_b().
