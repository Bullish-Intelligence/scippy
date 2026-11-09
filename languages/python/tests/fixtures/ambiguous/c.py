import sys

if sys.argv:
    from a import f
#                 ^ reference scippy python workspace . a/f().
else:
    from b import f
#                 ^ reference scippy python workspace . b/f().

value = f()
