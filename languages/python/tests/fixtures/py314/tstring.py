from base import render

name = "world"
width = 10
plain = t"hello {name}"
spec = rt"a\d {render(name)!r:>{width}} {width=}"
nested = t"outer {f"inner {name}"} {t"deep {width}"}"
