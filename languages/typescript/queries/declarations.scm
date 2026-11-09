; Declarations, import sites, members, and supertypes of TypeScript and TSX.
;
; `@name` marks the identifier. `@definition.<kind>` marks the whole declaration.
; The patterns follow the binding rules of the Stack Graphs TypeScript crate:
; each identifier that the rules define as a definition has one pattern here.
; A name in an import or export clause is a binding, not a declaration. The rules define it,
; and the reconciler replaces it with the final definition.

; ---- functions, classes, interfaces, enums, aliases, namespaces ----

(function_declaration
  name: (identifier) @name) @definition.function
(generator_function_declaration
  name: (identifier) @name) @definition.function
(function_signature
  name: (identifier) @name) @definition.function

(class_declaration
  name: (type_identifier) @name) @definition.class
(abstract_class_declaration
  name: (type_identifier) @name) @definition.class
(interface_declaration
  name: (type_identifier) @name) @definition.interface
(enum_declaration
  name: (identifier) @name) @definition.enum
(type_alias_declaration
  name: (type_identifier) @name) @definition.type
(internal_module
  name: (identifier) @name) @definition.namespace
(module
  name: (identifier) @name) @definition.namespace

; The member of an enum: `A` and `B` in `enum E { A, B = 2 }`.
(enum_body
  name: (property_identifier) @name @definition.field)
(enum_assignment
  name: (property_identifier) @name @definition.field)

; ---- default exports ----
;
; The binding rules end a path at the whole statement of `export default`. The anchor joins that
; statement to the declaration that it exports. An expression has no declaration of its own.
; The statement declares a variable named `default`, at the keyword.

(export_statement
  "default"
  declaration: [
    (function_declaration name: (identifier) @name) @definition.function
    (generator_function_declaration name: (identifier) @name) @definition.function
    (class_declaration name: (type_identifier) @name) @definition.class
    (abstract_class_declaration name: (type_identifier) @name) @definition.class
    (interface_declaration name: (type_identifier) @name) @definition.interface
  ]) @anchor
(export_statement
  "default" @name
  value: [
    (array) (as_expression) (assignment_expression) (augmented_assignment_expression)
    (await_expression) (binary_expression) (call_expression) (class) (false)
    (instantiation_expression) (member_expression) (meta_property)
    (new_expression) (non_null_expression) (null) (number) (object)
    (parenthesized_expression) (regex) (satisfies_expression) (string) (subscript_expression)
    (template_string) (ternary_expression) (this) (true) (unary_expression)
    (undefined) (update_expression) (yield_expression)
    (arrow_function) (function_expression) (generator_function)
  ]) @definition.variable @anchor
; `export default name` names what `name` names.
(export_statement
  "default"
  value: (identifier) @alias.target) @alias.name

; ---- members of classes and interfaces ----

(public_field_definition
  name: [(property_identifier) (private_property_identifier)] @name) @definition.field
(class_body
  (method_definition
    name: [(property_identifier) (private_property_identifier)] @name) @definition.function)
(abstract_method_signature
  name: [(property_identifier) (private_property_identifier)] @name) @definition.function

; The members of an interface and of an object type. An object type that a type alias names
; has the members of the alias. Any other object type is an anonymous scope, so its members are
; local.
(interface_body
  (property_signature
    name: [(property_identifier) (private_property_identifier)] @name) @definition.field)
(interface_body
  (method_signature
    name: [(property_identifier) (private_property_identifier)] @name) @definition.function)
(object_type
  (property_signature
    name: [(property_identifier) (private_property_identifier)] @name) @definition.field)
(object_type
  (method_signature
    name: [(property_identifier) (private_property_identifier)] @name) @definition.function)

; A parameter property, such as `constructor(private x: number)`, is a field of the class and a
; parameter of the constructor. The binding rules define the parameter. The overlay of the
; rules in the pack defines the field. Only the field gets a declaration here.
((required_parameter
  pattern: (identifier) @name) @definition.field
  (#match? @definition.field "^\\s*(@[\\w.]+(\\([^)]*\\))?\\s*)*(public|private|protected|readonly|override)\\b"))
((optional_parameter
  pattern: (identifier) @name) @definition.field
  (#match? @definition.field "^\\s*(@[\\w.]+(\\([^)]*\\))?\\s*)*(public|private|protected|readonly|override)\\b"))

; ---- variables ----

; A name that a pattern binds: a variable, a loop variable, or a destructured name.
; A `const f = () => {}` is a variable. Its arrow function is an anonymous scope.
(variable_declarator
  name: (identifier) @name) @definition.variable
(for_in_statement
  ["var" "let" "const"]
  left: (identifier) @name) @definition.variable
(catch_clause
  parameter: (identifier) @name @definition.parameter)
(shorthand_property_identifier_pattern) @name @definition.variable
(pair_pattern
  value: (identifier) @name @definition.variable)
(array_pattern
  (identifier) @name @definition.variable)
(assignment_pattern
  left: (identifier) @name @definition.variable)
(rest_pattern
  (identifier) @name @definition.variable)

; ---- parameters ----
;
((required_parameter
  pattern: (identifier) @name) @definition.parameter
  (#not-match? @definition.parameter "^\\s*(@[\\w.]+(\\([^)]*\\))?\\s*)*(public|private|protected|readonly|override)\\b"))
((optional_parameter
  pattern: (identifier) @name) @definition.parameter
  (#not-match? @definition.parameter "^\\s*(@[\\w.]+(\\([^)]*\\))?\\s*)*(public|private|protected|readonly|override)\\b"))
(arrow_function
  parameter: (identifier) @name @definition.parameter)

; ---- type parameters ----

(type_parameter
  name: (type_identifier) @name) @definition.typeparameter

; ---- nodes that hold declarations and have no name ----

(arrow_function) @scope
(function_expression) @scope
(generator_function) @scope
(class) @scope
(object
  (method_definition) @scope)
(class_static_block) @scope
; An object type that no type alias names is an anonymous scope.
(adding_type_annotation (object_type) @scope)
(array_type (object_type) @scope)
(as_expression (object_type) @scope)
(conditional_type (object_type) @scope)
(constraint (object_type) @scope)
(constructor_type (object_type) @scope)
(default_type (object_type) @scope)
(function_type (object_type) @scope)
(implements_clause (object_type) @scope)
(index_signature (object_type) @scope)
(infer_type (object_type) @scope)
(intersection_type (object_type) @scope)
(lookup_type (object_type) @scope)
(mapped_type_clause (object_type) @scope)
(omitting_type_annotation (object_type) @scope)
(opting_type_annotation (object_type) @scope)
(optional_type (object_type) @scope)
(parenthesized_type (object_type) @scope)
(readonly_type (object_type) @scope)
(rest_type (object_type) @scope)
(satisfies_expression (object_type) @scope)
(tuple_type (object_type) @scope)
(type_annotation (object_type) @scope)
(type_arguments (object_type) @scope)
(type_predicate (object_type) @scope)
(union_type (object_type) @scope)
(function_type) @scope
(constructor_type) @scope
(call_signature) @scope
(construct_signature) @scope

; ---- the module ----

(program) @module

; ---- import sites ----

(import_specifier
  name: (identifier) @import)
(export_specifier
  name: (identifier) @import)

; ---- names that an import or an export binds ----
;
; The binding rules define such a name and do not connect a reference to it. The name takes the
; target of the node that it imports. The statement of a default import carries the reference of
; the binding rules, so it stays out of the output.

; import * as ns from "m"
(import_statement
  (import_clause (namespace_import (identifier) @alias.name @import))
  source: (string) @alias.target)
; import ns = require("m")
(import_require_clause
  (identifier) @alias.name @import
  source: (string) @alias.target)
; export * as ns from "m"
(export_statement
  (namespace_export (identifier) @alias.name @import)
  source: (string) @alias.target)
; import def from "m"
(import_statement
  (import_clause (identifier) @alias.name @import)) @alias.target @hidden
; import { a as b } from "m" and export { a as b }
(import_specifier
  name: (identifier) @alias.target
  alias: (identifier) @alias.name @import)
(export_specifier
  name: (identifier) @alias.target
  alias: (identifier) @alias.name @import)

; ---- members of a value: the `x` in `a.x` ----

(member_expression
  property: [(property_identifier) (private_property_identifier)] @member)
(nested_type_identifier
  name: (type_identifier) @member)
(nested_identifier
  property: (property_identifier) @member)

; ---- supertypes in the header of a class or an interface ----

(extends_clause
  value: [
    (identifier) @supertype
    (member_expression property: (property_identifier) @supertype)
  ])
(implements_clause
  [
    (type_identifier) @supertype
    (nested_type_identifier name: (type_identifier) @supertype)
    (generic_type name: (type_identifier) @supertype)
    (generic_type name: (nested_type_identifier name: (type_identifier) @supertype))
  ])
(extends_type_clause
  type: [
    (type_identifier) @supertype
    (nested_type_identifier name: (type_identifier) @supertype)
    (generic_type name: (type_identifier) @supertype)
    (generic_type name: (nested_type_identifier name: (type_identifier) @supertype))
  ])
