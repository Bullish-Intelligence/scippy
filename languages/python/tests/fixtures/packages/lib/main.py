from pkg import helper
#               ^^^^^^ reference scippy python workspace . lib/pkg/core/helper().
from pkg.core import Box
#                    ^^^ reference scippy python workspace . lib/pkg/core/Box#

box = Box()
#     ^^^ reference scippy python workspace . lib/pkg/core/Box#
x = helper()
#   ^^^^^^ reference scippy python workspace . lib/pkg/core/helper().
y = box.get()
#       ^^^ reference scippy python workspace . lib/pkg/core/Box#get().
