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

; The name of an assignment expression: the `n` in `(n := f())`.
(named_expression
  name: (identifier) @name @definition.variable)

; A type alias: `type Name = value` and `type Name[T] = value`.
(type_alias_statement
  . (type [
    (identifier) @name
    (generic_type . (identifier) @name)
  ])) @definition.type

; The type parameters of a class, a function, and a type alias: the `T` in `class C[T: Base]`.
; The bound is no declaration.
(class_definition
  type_parameters: (type_parameter
    (type [
      (identifier) @name
      (constrained_type . (type (identifier) @name))
      (splat_type (identifier) @name)
    ]) @definition.typeparameter))
(function_definition
  type_parameters: (type_parameter
    (type [
      (identifier) @name
      (constrained_type . (type (identifier) @name))
      (splat_type (identifier) @name)
    ]) @definition.typeparameter))
(type_alias_statement
  . (type (generic_type (type_parameter
    (type [
      (identifier) @name
      (constrained_type . (type (identifier) @name))
      (splat_type (identifier) @name)
    ]) @definition.typeparameter))))

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

; The module of the document. The binding rules define it at the start of the root node.
(module) @module

; Module names in an import statement: `a` and `b` in `import a.b`, `import a.b as c`,
; and `from a.b import x`. Each one names a module.
(import_statement
  name: (dotted_name (identifier) @import))
(import_statement
  name: (aliased_import name: (dotted_name (identifier) @import)))
(import_from_statement
  module_name: (dotted_name (identifier) @import))
(import_from_statement
  module_name: (relative_import (dotted_name (identifier) @import)))

; Imported names: `from m import a` and `from m import a as b`.
(import_from_statement
  name: (dotted_name (identifier) @import))
(import_from_statement
  name: (aliased_import name: (dotted_name (identifier) @import)))

; Members of a value: the `x` in `a.x`. The path of such a reference passes the definition of the
; object `a`. Every other name is a plain name. The reconciler applies a stricter
; nearest-binding rule to a plain name.
(attribute
  attribute: (identifier) @member)

; Keyword argument names: the `x` in `f(x=1)`. The name is a reference to a parameter of the
; callee. The path of the reference starts at the callee, so the reconciler treats it like a member.
(call
  arguments: (argument_list
    (keyword_argument
      name: (identifier) @keyword)))

; Base classes in the header of a class: `Base`, `pkg.Base`, `Base[T]`, and `pkg.Base[T]`.
; The identifier is also a plain reference or a member, so the reconciler resolves it.
(class_definition
  superclasses: (argument_list
    [(identifier) @supertype
     (attribute attribute: (identifier) @supertype)
     (subscript value: (identifier) @supertype)
     (subscript value: (attribute attribute: (identifier) @supertype))]))
