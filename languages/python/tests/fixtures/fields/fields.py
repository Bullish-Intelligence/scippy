class State:
    id: str
#   ^^ definition scippy python workspace . fields/State#id.
    count = 0
#   ^^^^^ definition scippy python workspace . fields/State#count.
    limit = 1
    label = count
#   ^^^^^ definition scippy python workspace . fields/State#label.
#           ^^^^^ reference scippy python workspace . fields/State#count.

    def __init__(self, id: str | None, temporary: bool = False) -> None:
        if not temporary:
            assert id, "no id"
#                  ^^ reference scippy python workspace . fields/State#__init__().(id)
        print(id)
#             ^^ reference scippy python workspace . fields/State#__init__().(id)
        self.id = id or "main"
#       ^^^^ reference scippy python workspace . fields/State#__init__().(self)
#            ^^ definition scippy python workspace . fields/State#id.
#                 ^^ reference scippy python workspace . fields/State#__init__().(id)

    def show(self):
        print(count)
        print(limit)
#             ^^^^^ reference scippy python workspace . fields/limit.
        return self.id
#              ^^^^ reference scippy python workspace . fields/State#show().(self)
#                   ^^ reference scippy python workspace . fields/State#id.


limit = 3
