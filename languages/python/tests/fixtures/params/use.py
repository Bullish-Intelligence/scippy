from lib import Box, make

box = Box(size=1, label="a")
box.resize(size=2, force=True)
make(width=1, height=2)
Box.unit(label="b")
