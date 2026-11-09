import lib
# <- definition scippy python workspace . app/
#      ^^^ reference scippy python workspace . lib/
import lib.util as u
#      ^^^ reference scippy python workspace . lib/
#          ^^^^ reference scippy python workspace . lib/util/
from lib import util
#    ^^^ reference scippy python workspace . lib/
#               ^^^^ reference scippy python workspace . lib/util/
from lib.sub import leaf as lf
#    ^^^ reference scippy python workspace . lib/
#        ^^^ reference scippy python workspace . lib/sub/
#                   ^^^^ reference scippy python workspace . lib/sub/leaf/
from lib.sub.leaf import X
#                        ^ reference scippy python workspace . lib/sub/leaf/X.

a = lib.util.VALUE
#       ^^^^ reference scippy python workspace . lib/util/
#            ^^^^^ reference scippy python workspace . lib/util/VALUE.
b = u.VALUE
#   ^ reference scippy python workspace . lib/util/
#     ^^^^^ reference scippy python workspace . lib/util/VALUE.
d = lf.X
#   ^^ reference scippy python workspace . lib/sub/leaf/
#      ^ reference scippy python workspace . lib/sub/leaf/X.
e = util.VALUE
#   ^^^^ reference scippy python workspace . lib/util/
#        ^^^^^ reference scippy python workspace . lib/util/VALUE.
f = X
