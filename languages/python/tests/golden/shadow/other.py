 def process(value):
#^ definition scippy python workspace . other/
#  kind Module
#  display_name other
#    ^^^^^^^ definition scippy python workspace . other/process().
#            kind Function
#            display_name process
#            ^^^^^ definition scippy python workspace . other/process().(value)
#                  kind Parameter
#                  display_name value
 #   ^^^^^^^ definition scippy python workspace . other/process().
     return value
#           ^^^^^ reference scippy python workspace . other/process().(value)
 
 
 def unrelated():
#    ^^^^^^^^^ definition scippy python workspace . other/unrelated().
#              kind Function
#              display_name unrelated
 #   ^^^^^^^^^ definition scippy python workspace . other/unrelated().
     return process(3)
#           ^^^^^^^ reference scippy python workspace . other/process().
 #          ^^^^^^^ reference scippy python workspace . other/process().
 
