; candela highlighting.
;
; Capture names follow the Neovim / Helix vocabulary. Zed ships its own copy of
; this file under editors/zed, because Zed reads queries from the
; extension rather than from the grammar repository and its theme keys differ.
;
; Patterns that could match the same node are partitioned with predicates
; instead of relying on query order, since editors disagree about whether the
; first or the last matching pattern wins.

; Keywords

[
  "import"
  "as"
] @keyword.import

"fn" @keyword.function

"let" @keyword

"return" @keyword.return

[
  "if"
  "else"
  "match"
] @keyword.conditional

[
  "while"
  "for"
  "in"
  "loop"
  "break"
  "continue"
] @keyword.repeat

[
  "try"
  "catch"
] @keyword.exception

[
  "struct"
  "enum"
  "impl"
] @keyword.type

; `dylib` and `host` open a block of foreign function signatures.
[
  "dylib"
  "host"
] @keyword.directive

; Types

(type_identifier) @type

((type_identifier) @type.builtin
  (#any-of? @type.builtin "int" "float" "string" "bool" "null" "any"))

(qualified_type
  module: (type_identifier) @module)

(struct_declaration
  name: (identifier) @type)

(enum_declaration
  name: (identifier) @type)

(struct_literal
  name: (identifier) @type)

(enum_variant
  name: (identifier) @constructor)

; Functions

(function_declaration
  name: (identifier) @function)

(function_signature
  name: (identifier) @function)

(call_expression
  function: (identifier) @function.call)

(call_expression
  function: (qualified_identifier
    name: (identifier) @function.call))

(method_call_expression
  method: (identifier) @function.method.call)

(method_call_expression
  method: (qualified_identifier
    name: (identifier) @function.method.call))

; The built-ins need no import and are always in scope.
((call_expression
  function: (identifier) @function.builtin)
  (#any-of? @function.builtin
    "print" "type" "float" "int" "str" "bool" "input" "range" "the_answer"
    "argv" "exit" "throw"))

; Macros

(macro_invocation
  name: (identifier) @function.macro)

(macro_invocation
  "!(" @punctuation.special)

; What stands between the parentheses is the host's syntax, so it is coloured
; as raw text rather than run through candela's rules.
(macro_body) @markup.raw

; Variables

(parameter
  name: (identifier) @variable.parameter)

(field_declaration
  name: (identifier) @variable.member)

(field_initializer
  name: (identifier) @variable.member)

(field_expression
  field: (identifier) @variable.member)

(qualified_identifier
  module: (identifier) @module)

(import_declaration
  alias: (identifier) @module)

(identifier) @variable

; Literals

(integer_literal) @number

(float_literal) @number.float

(string_literal) @string

(escape_sequence) @string.escape

(boolean_literal) @boolean

(null_literal) @constant.builtin

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
((comment) @comment.documentation
  (#match? @comment.documentation "^///"))

((comment) @comment
  (#not-match? @comment "^///"))
