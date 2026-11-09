 from lib import grüße, 日本
#^ definition scippy python workspace . use/
#  kind Module
#  display_name use
#     ^^^ reference scippy python workspace . lib/
#                ^^^^^^^ reference scippy python workspace . lib/`grüße`().
#                         ^^^^^^ reference scippy python workspace . lib/`日本`.
 #               ^^^^^^^ reference scippy python workspace . lib/`grüße`().
 #                        ^^^^^^ reference scippy python workspace . lib/`日本`.
 x = grüße("é")  # é 日本 grüße
#^ definition scippy python workspace . use/x.
#  kind Variable
#  display_name x
#    ^^^^^^^ reference scippy python workspace . lib/`grüße`().
 #   ^^^^^^^ reference scippy python workspace . lib/`grüße`().
 naïve = 日本
#^^^^^^ definition scippy python workspace . use/`naïve`.
#       kind Variable
#       display_name naïve
#         ^^^^^^ reference scippy python workspace . lib/`日本`.
 #        ^^^^^^ reference scippy python workspace . lib/`日本`.
 y = grüße
#^ definition scippy python workspace . use/y.
#  kind Variable
#  display_name y
#    ^^^^^^^ reference scippy python workspace . lib/`grüße`().
 #   ^^^^^^^ reference scippy python workspace . lib/`grüße`().
 
