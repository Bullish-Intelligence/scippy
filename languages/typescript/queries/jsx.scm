;
; JSX patterns. Only the TSX grammar has these nodes. The core adds them to the query of the pack.
;
; A tag that starts with a lowercase letter is an intrinsic element, such as `<div>`. It never
; names a variable. The binding rules read it as a name, so a variable named `div` would be its
; target. The reference gets no occurrence.

(jsx_opening_element
  name: (identifier) @hidden
  (#match? @hidden "^[a-z]"))
(jsx_closing_element
  name: (identifier) @hidden
  (#match? @hidden "^[a-z]"))
(jsx_self_closing_element
  name: (identifier) @hidden
  (#match? @hidden "^[a-z]"))
