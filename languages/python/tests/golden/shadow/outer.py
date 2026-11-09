 def process(value):
#^ definition scippy python workspace . outer/
#  kind Module
#  display_name outer
#    ^^^^^^^ definition scippy python workspace . outer/process().
#            kind Function
#            display_name process
#            ^^^^^ definition scippy python workspace . outer/process().(value)
#                  kind Parameter
#                  display_name value
 #   ^^^^^^^ definition scippy python workspace . outer/process().
 #           ^^^^^ definition scippy python workspace . outer/process().(value)
     return value
#           ^^^^^ reference scippy python workspace . outer/process().(value)
 #          ^^^^^ reference scippy python workspace . outer/process().(value)
 
 
 def local_shadow(process):
#    ^^^^^^^^^^^^ definition scippy python workspace . outer/local_shadow().
#                 kind Function
#                 display_name local_shadow
#                 ^^^^^^^ definition scippy python workspace . outer/local_shadow().(process)
#                         kind Parameter
#                         display_name process
 #   ^^^^^^^^^^^^ definition scippy python workspace . outer/local_shadow().
 #                ^^^^^^^ definition scippy python workspace . outer/local_shadow().(process)
     return process
#           ^^^^^^^ reference scippy python workspace . outer/local_shadow().(process)
 #          ^^^^^^^ reference scippy python workspace . outer/local_shadow().(process)
 
 
 def rebinding():
#    ^^^^^^^^^ definition scippy python workspace . outer/rebinding().
#              kind Function
#              display_name rebinding
 #   ^^^^^^^^^ definition scippy python workspace . outer/rebinding().
     process = 1
#    ^^^^^^^ definition local 0
#            kind Variable
#            display_name process
 #   ^^^^^^^ definition local 0
     return process
#           ^^^^^^^ reference local 0
 #          ^^^^^^^ reference local 0
 
 
 def calls_outer():
#    ^^^^^^^^^^^ definition scippy python workspace . outer/calls_outer().
#                kind Function
#                display_name calls_outer
 #   ^^^^^^^^^^^ definition scippy python workspace . outer/calls_outer().
     return process(2)
#           ^^^^^^^ reference scippy python workspace . outer/process().
 #          ^^^^^^^ reference scippy python workspace . outer/process().
 
