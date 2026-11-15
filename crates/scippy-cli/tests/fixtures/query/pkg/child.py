import sys

from pkg.base import Base
from pkg.café import bonjour

if sys.argv:
    from pkg.left import pick
else:
    from pkg.right import pick


class Child(Base):
    def greet(self):
        return bonjour()


value = pick()
missing = undefined_name()
