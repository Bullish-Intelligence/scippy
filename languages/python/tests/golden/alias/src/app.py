 from services import process as run
#^ definition scippy python workspace . src/app/
#  kind Module
#  display_name app
#     ^^^^^^^^ reference scippy python workspace . src/services/
#                     ^^^^^^^ reference scippy python workspace . src/services/process().
 #                    ^^^^^^^ reference scippy python workspace . src/services/process().
 result = run("hello")
#^^^^^^ definition scippy python workspace . src/app/result.
#       kind Variable
#       display_name result
#         ^^^ reference scippy python workspace . src/services/process().
 # <- definition scippy python workspace . src/app/result.
 #        ^^^ reference scippy python workspace . src/services/process().
 
