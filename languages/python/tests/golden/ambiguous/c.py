 import sys
#^ definition scippy python workspace . c/
#  kind Module
#  display_name c
 
 if sys.argv:
     from a import f
#         ^ reference scippy python workspace . a/
#                  ^ reference scippy python workspace . a/f().
 #                 ^ reference scippy python workspace . a/f().
 else:
     from b import f
#         ^ reference scippy python workspace . b/
#                  ^ reference scippy python workspace . b/f().
 #                 ^ reference scippy python workspace . b/f().
 
 value = f()
#^^^^^ definition scippy python workspace . c/value.
#      kind Variable
#      display_name value
 
