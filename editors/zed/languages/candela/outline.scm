(function_declaration
  "fn" @context
  name: (identifier) @name) @item

(struct_declaration
  "struct" @context
  name: (identifier) @name) @item

(enum_declaration
  "enum" @context
  name: (identifier) @name) @item

(impl_block
  "impl" @context
  type: (type_identifier) @name) @item

(dylib_block
  "dylib" @context
  path: (string_literal) @name) @item

(host_block
  "host" @context
  namespace: (string_literal) @name) @item
