from lib import grüße, 日本
#               ^^^^^^^ reference scippy python workspace . lib/`grüße`().
#                        ^^^^^^ reference scippy python workspace . lib/`日本`.
x = grüße("é")  # é 日本 grüße
#   ^^^^^^^ reference scippy python workspace . lib/`grüße`().
naïve = 日本
#        ^^^^^^ reference scippy python workspace . lib/`日本`.
y = grüße
#   ^^^^^^^ reference scippy python workspace . lib/`grüße`().
