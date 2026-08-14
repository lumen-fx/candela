; candela highlighting for Zed.
;
; Zed reads queries from the extension rather than from the grammar
; repository, and its theme keys differ from the Neovim / Helix vocabulary, so
; this is a separate file from editors/tree-sitter/queries/highlights.scm.
; Keep the two in step when the grammar changes.

; Keywords

[
  "as"
  "break"
  "catch"
  "continue"
  "dylib"
  "else"
  "enum"
  "fn"
  "for"
  "host"
  "if"
  "impl"
  "import"
  "in"
  "let"
  "loop"
  "match"
  "return"
  "struct"
  "try"
  "while"
] @keyword

; Types

(type_identifier) @type

(struct_declaration
  name: (identifier) @type)

(enum_declaration
  name: (identifier) @type)

(struct_literal
  name: (identifier) @type)

(enum_variant
  name: (identifier) @variant)

; Functions

(function_declaration
  name: (identifier) @function.definition)

(function_signature
  name: (identifier) @function.definition)

(call_expression
  function: (identifier) @function)

(call_expression
  function: (qualified_identifier
    name: (identifier) @function))

(method_call_expression
  method: (identifier) @function)

(method_call_expression
  method: (qualified_identifier
    name: (identifier) @function))

; The built-ins need no import and are always in scope.
((call_expression
  function: (identifier) @function.builtin)
  (#any-of? @function.builtin
    "print" "type" "float" "int" "str" "bool" "input" "range" "the_answer"
    "argv" "exit" "throw"))

; Variables

(parameter
  name: (identifier) @variable.parameter)

(field_declaration
  name: (identifier) @property)

(field_initializer
  name: (identifier) @property)

(field_expression
  field: (identifier) @property)

(identifier) @variable

; Literals

(integer_literal) @number

(float_literal) @number

(string_literal) @string

(escape_sequence) @string.escape

(boolean_literal) @boolean

(null_literal) @constant

; Operators and punctuation

(variadic_parameter) @operator

[
  "+"
  "-"
  "*"
  "/"
  "%"
  "^"
  "=="
  "!="
  "<"
  "<="
  ">"
  ">="
  "&&"
  "||"
  "!"
  "="
  "+="
  "-="
  "*="
  "/="
  "%="
  "^="
  ".."
  "->"
  "=>"
  "|"
] @operator

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

[
  ","
  ";"
  ":"
  "::"
  "."
] @punctuation.delimiter

; Comments. candela has one comment form, the line comment; a `///` comment is
; an ordinary comment that the standard library uses to document what follows.

((comment) @comment.doc
  (#match? @comment.doc "^///"))

((comment) @comment
  (#not-match? @comment "^///"))
