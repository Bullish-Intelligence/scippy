class State:
    id: str
#   ^^ definition scippy python workspace . fields/State#id.
    count = 0
#   ^^^^^ definition scippy python workspace . fields/State#count.
    label = count
#   ^^^^^ definition scippy python workspace . fields/State#label.
#           ^^^^^ reference scippy python workspace . fields/State#count.

    def __init__(self, id: str | None, temporary: bool = False) -> None:
        if not temporary:
            assert id, "no id"
#                  ^^ reference local 1
        print(id)
#             ^^ reference local 1
        self.id = id or "main"
#       ^^^^ reference local 0
#            ^^ definition scippy python workspace . fields/State#id.
#                 ^^ reference local 1

    def show(self):
        print(count)
        return self.id
#              ^^^^ reference local 3
#                   ^^ reference scippy python workspace . fields/State#id.
