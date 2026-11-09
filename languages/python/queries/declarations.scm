; Declarations and import sites of Python.
;
; `@name` marks the identifier. `@definition.<kind>` marks the whole declaration.
; The patterns follow the binding rules of the Stack Graphs Python crate:
; each identifier that the rules define as a definition has one pattern here.
; Names in import statements are bindings, not declarations. The rules define them, and
; the reconciler replaces them with the final definition.

(function_definition
  name: (identifier) @name) @definition.function

(class_definition
  name: (identifier) @name) @definition.class

; Names that code binds with an assignment, a loop, a `with`, an `except`, or a walrus.
(pattern/identifier) @name @definition.variable

; Names that a pattern binds: `case` patterns and `as` patterns.
(as_pattern
  alias: (as_pattern_target (identifier) @name @definition.variable))
(keyword_pattern
  (identifier) @name @definition.variable)
(splat_pattern
  (identifier) @name @definition.variable)

; Instance attributes that a method binds: `self.x = value`.
(pattern/attribute
  object: (identifier) @receiver
  attribute: (identifier) @name @definition.field
  (#eq? @receiver "self"))

; Parameters.
(parameter/identifier) @name @definition.parameter
(parameter/default_parameter
  name: (identifier) @name @definition.parameter)
(parameter/typed_default_parameter
  name: (identifier) @name @definition.parameter)
(parameter/typed_parameter
  . (identifier) @name @definition.parameter)
(parameter/typed_parameter
  . (list_splat_pattern (identifier) @name @definition.parameter))
(parameter/typed_parameter
  . (dictionary_splat_pattern (identifier) @name @definition.parameter))
(parameter/list_splat_pattern
  (identifier) @name @definition.parameter)
(parameter/dictionary_splat_pattern
  (identifier) @name @definition.parameter)

; Imported names: `from m import a` and `from m import a as b`.
(import_from_statement
  name: (dotted_name (identifier) @import))
(import_from_statement
  name: (aliased_import name: (dotted_name (identifier) @import)))
